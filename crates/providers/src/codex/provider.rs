use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use tracing::{debug, info, warn};

use brim_core::confidence::Labeled;
use brim_core::error::CoreError;
use brim_core::models::{AuthState, PlanInfo, ProviderId, QuotaBucket, UsageSnapshot};
use brim_core::provider::{
    AuthFlowKind, AuthSessionCommand, AuthSessionState, AuthSessionView, Provider,
    ProviderAuthSession,
};
use brim_core::time_window::TimeWindow;

use brim_auth::local_files;

/// Codex / ChatGPT provider implementation.
///
/// Strategy order:
/// 1. CLI JSON-RPC via `codex app-server` (newline-delimited JSON-RPC over stdio)
/// 2. Local auth.json JWT fallback (plan info only, no quota)
pub struct CodexProvider {
    /// Kept for future HTTP-based fallback strategies.
    #[allow(dead_code)]
    http: Arc<reqwest::Client>,
}

impl CodexProvider {
    pub fn new(http: Arc<reqwest::Client>) -> Self {
        Self { http }
    }

    /// Try to read the Codex CLI auth.json file.
    fn read_auth_file(&self) -> Option<CodexAuth> {
        let path = local_files::find_codex_auth_file()?;
        debug!("Found codex auth file at {}", path.display());
        match local_files::read_json_file::<CodexAuth>(&path) {
            Ok(auth) => Some(auth),
            Err(e) => {
                warn!("Failed to parse codex auth.json: {}", e);
                None
            }
        }
    }

    /// Fetch usage via `codex app-server` JSON-RPC over stdio.
    ///
    /// Protocol:
    /// - Newline-delimited JSON (NOT LSP Content-Length framing)
    /// - Handshake: send `initialize`, then `initialized` notification
    /// - Then query `account/read` and `account/rateLimits/read`
    async fn fetch_via_cli_rpc(&self) -> Result<UsageSnapshot, CoreError> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::process::Command;

        // Check if codex binary exists
        let which = Command::new("which")
            .arg("codex")
            .output()
            .await
            .map_err(|e| CoreError::FetchFailed {
                provider: "codex".into(),
                reason: format!("cannot locate codex binary: {}", e),
            })?;

        if !which.status.success() {
            return Err(CoreError::FetchFailed {
                provider: "codex".into(),
                reason: "codex CLI not found in PATH".into(),
            });
        }

        // Spawn codex app-server
        let mut child = Command::new("codex")
            .arg("app-server")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| CoreError::FetchFailed {
                provider: "codex".into(),
                reason: format!("failed to spawn codex app-server: {}", e),
            })?;

        let stdin = child.stdin.take().ok_or_else(|| CoreError::FetchFailed {
            provider: "codex".into(),
            reason: "failed to get stdin of codex app-server".into(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| CoreError::FetchFailed {
            provider: "codex".into(),
            reason: "failed to get stdout of codex app-server".into(),
        })?;

        let mut writer = tokio::io::BufWriter::new(stdin);
        let mut reader = BufReader::new(stdout);

        // Helper: send a newline-delimited JSON message
        async fn send_msg(
            writer: &mut tokio::io::BufWriter<tokio::process::ChildStdin>,
            msg: &serde_json::Value,
        ) -> Result<(), CoreError> {
            let line = serde_json::to_string(msg).map_err(|e| CoreError::FetchFailed {
                provider: "codex".into(),
                reason: format!("JSON serialize error: {}", e),
            })?;
            writer
                .write_all(line.as_bytes())
                .await
                .map_err(|e| CoreError::FetchFailed {
                    provider: "codex".into(),
                    reason: format!("write to app-server failed: {}", e),
                })?;
            writer
                .write_all(b"\n")
                .await
                .map_err(|e| CoreError::FetchFailed {
                    provider: "codex".into(),
                    reason: format!("write newline failed: {}", e),
                })?;
            writer.flush().await.map_err(|e| CoreError::FetchFailed {
                provider: "codex".into(),
                reason: format!("flush failed: {}", e),
            })?;
            Ok(())
        }

        // Helper: read a JSON-RPC response with the given id, with timeout.
        // Skips notification lines (lines without "id" or with different id).
        async fn read_response(
            reader: &mut BufReader<tokio::process::ChildStdout>,
            expected_id: u64,
            timeout_secs: u64,
        ) -> Result<serde_json::Value, CoreError> {
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
            let mut line_buf = String::new();
            loop {
                line_buf.clear();
                let read_fut = reader.read_line(&mut line_buf);
                let n = tokio::time::timeout_at(deadline, read_fut)
                    .await
                    .map_err(|_| CoreError::FetchFailed {
                        provider: "codex".into(),
                        reason: format!("timeout waiting for response id={}", expected_id),
                    })?
                    .map_err(|e| CoreError::FetchFailed {
                        provider: "codex".into(),
                        reason: format!("read from app-server failed: {}", e),
                    })?;
                if n == 0 {
                    return Err(CoreError::FetchFailed {
                        provider: "codex".into(),
                        reason: "app-server closed stdout before responding".into(),
                    });
                }
                let trimmed = line_buf.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Try to parse as JSON
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    // Check if this is the response we want
                    if let Some(id) = val.get("id").and_then(|v| v.as_u64()) {
                        if id == expected_id {
                            return Ok(val);
                        }
                    }
                    // Otherwise it's a notification or different response; skip it
                    debug!(
                        "codex app-server: skipping line: {}",
                        &trimmed[..trimmed.len().min(200)]
                    );
                } else {
                    debug!(
                        "codex app-server: non-JSON line: {}",
                        &trimmed[..trimmed.len().min(200)]
                    );
                }
            }
        }

        // 1. Send initialize request (id=1)
        let init_req = serde_json::json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "brim",
                    "version": "0.1.0"
                },
                "capabilities": {
                    "experimentalApi": true
                }
            }
        });
        send_msg(&mut writer, &init_req).await?;

        // Read initialize response
        let init_resp = read_response(&mut reader, 1, 10).await?;
        debug!(
            "codex initialize response: {}",
            serde_json::to_string(&init_resp)
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect::<String>()
        );

        // 2. Send initialized notification (no id — it's a notification)
        let initialized_notif = serde_json::json!({
            "method": "initialized"
        });
        send_msg(&mut writer, &initialized_notif).await?;

        // Small delay to let the server process the notification
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // 3. Send account/read (id=2)
        let account_req = serde_json::json!({
            "id": 2,
            "method": "account/read",
            "params": {}
        });
        send_msg(&mut writer, &account_req).await?;
        let account_resp = read_response(&mut reader, 2, 10).await?;
        debug!(
            "codex account/read response: {}",
            serde_json::to_string(&account_resp)
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect::<String>()
        );

        // 4. Send account/rateLimits/read (id=3)
        let rate_req = serde_json::json!({
            "id": 3,
            "method": "account/rateLimits/read",
            "params": {}
        });
        send_msg(&mut writer, &rate_req).await?;
        let rate_resp = read_response(&mut reader, 3, 10).await?;
        debug!(
            "codex rateLimits response: {}",
            serde_json::to_string(&rate_resp)
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect::<String>()
        );

        // Kill the app-server process
        let _ = child.kill().await;

        // Parse account info
        let account: Option<RpcAccountResult> = account_resp
            .get("result")
            .and_then(|r| serde_json::from_value(r.clone()).ok());

        // Parse rate limits
        let rate_limits: Option<RpcRateLimitsResult> = rate_resp
            .get("result")
            .and_then(|r| serde_json::from_value(r.clone()).ok());

        let rate_data = rate_limits.ok_or_else(|| CoreError::FetchFailed {
            provider: "codex".into(),
            reason: "could not parse account/rateLimits/read response".into(),
        })?;

        Ok(self.build_snapshot_from_rpc(account, rate_data))
    }

    /// Build a `UsageSnapshot` from the real JSON-RPC responses.
    fn build_snapshot_from_rpc(
        &self,
        account: Option<RpcAccountResult>,
        rate_data: RpcRateLimitsResult,
    ) -> UsageSnapshot {
        let mut buckets = Vec::new();

        // Use the top-level rateLimits object
        if let Some(ref rl) = rate_data.rate_limits {
            self.push_buckets_from_rate_limit(rl, &mut buckets);
        }

        // Determine plan: prefer rateLimits.planType, fallback to account
        let plan_type = rate_data
            .rate_limits
            .as_ref()
            .and_then(|rl| rl.plan_type.clone())
            .or_else(|| {
                account
                    .as_ref()
                    .and_then(|a| a.account.as_ref())
                    .and_then(|a| a.plan_type.clone())
            });

        let plan = plan_type.map(|p| PlanInfo {
            name: Labeled::provider_local(p),
            tier: None,
        });

        UsageSnapshot {
            provider: ProviderId::Codex,
            fetched_at: Utc::now(),
            plan,
            buckets,
            source_strategy: "cli_rpc".into(),
            notes: vec!["Data from Codex app-server JSON-RPC (provider-local)".into()],
        }
    }

    /// Convert a single `RpcRateLimitEntry` (which has primary + secondary windows)
    /// into one or two `QuotaBucket`s.
    fn push_buckets_from_rate_limit(&self, rl: &RpcRateLimitEntry, buckets: &mut Vec<QuotaBucket>) {
        if let Some(ref primary) = rl.primary {
            let duration_secs = primary.window_duration_mins.unwrap_or(300) * 60;
            let hours = duration_secs / 3600;
            let label = if hours > 0 {
                format!("{}-hour window", hours)
            } else {
                format!("{}-min window", duration_secs / 60)
            };

            let mut window = if duration_secs <= 24 * 3600 {
                TimeWindow::session(label, duration_secs)
            } else {
                TimeWindow::weekly(label)
            };

            if let Some(ts) = primary.resets_at {
                if let Some(dt) = timestamp_to_datetime(ts) {
                    window = window.with_reset(dt);
                }
            }

            let used_pct = primary.used_percent.unwrap_or(0.0);
            buckets.push(QuotaBucket {
                metric: "primary".into(),
                label: "Primary (session)".into(),
                window,
                used: Some(Labeled::provider_local(used_pct)),
                limit: Some(Labeled::provider_local(100.0)),
                percent_remaining: Some(Labeled::provider_local(
                    ((100.0 - used_pct) / 100.0).clamp(0.0, 1.0),
                )),
            });
        }

        if let Some(ref secondary) = rl.secondary {
            let duration_secs = secondary.window_duration_mins.unwrap_or(10080) * 60;
            let days = duration_secs / 86400;
            let label = if days > 0 {
                format!("{}-day window", days)
            } else {
                format!("{}-hour window", duration_secs / 3600)
            };

            let mut window = if duration_secs >= 7 * 24 * 3600 {
                TimeWindow::weekly(label)
            } else {
                TimeWindow::session(label, duration_secs)
            };

            if let Some(ts) = secondary.resets_at {
                if let Some(dt) = timestamp_to_datetime(ts) {
                    window = window.with_reset(dt);
                }
            }

            let used_pct = secondary.used_percent.unwrap_or(0.0);
            buckets.push(QuotaBucket {
                metric: "secondary".into(),
                label: "Secondary (weekly)".into(),
                window,
                used: Some(Labeled::provider_local(used_pct)),
                limit: Some(Labeled::provider_local(100.0)),
                percent_remaining: Some(Labeled::provider_local(
                    ((100.0 - used_pct) / 100.0).clamp(0.0, 1.0),
                )),
            });
        }
    }
}

/// Convert a unix timestamp (seconds) to a `DateTime<Utc>`.
fn timestamp_to_datetime(ts: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(ts, 0).single()
}

#[async_trait]
impl Provider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn display_name(&self) -> &str {
        "Codex / ChatGPT"
    }

    async fn auth_state(&self) -> AuthState {
        match self.read_auth_file() {
            Some(auth) => {
                if auth.has_token() {
                    AuthState::Configured
                } else {
                    AuthState::NotConfigured
                }
            }
            None => AuthState::NotConfigured,
        }
    }

    async fn authenticate(&self) -> Result<AuthState, CoreError> {
        match self.read_auth_file() {
            Some(auth) => {
                if auth.has_token() {
                    info!("Codex auth file found with valid token");
                    Ok(AuthState::Authenticated)
                } else {
                    Ok(AuthState::Failed(
                        "auth.json exists but has no usable token".into(),
                    ))
                }
            }
            None => {
                info!("No codex auth file found. Run `codex` CLI to authenticate first.");
                Ok(AuthState::NotConfigured)
            }
        }
    }

    fn begin_auth_session(&self) -> Result<Box<dyn ProviderAuthSession>, CoreError> {
        Ok(Box::new(CodexAuthSession::new(self.http.clone())))
    }

    async fn fetch_usage(&self) -> Result<UsageSnapshot, CoreError> {
        // Strategy 1: Try CLI JSON-RPC (best source — real quota data)
        match self.fetch_via_cli_rpc().await {
            Ok(mut snapshot) => {
                // If app-server didn't return plan, try JWT fallback
                if snapshot.plan.is_none() {
                    if let Some(auth) = self.read_auth_file() {
                        if let Some(plan_name) = auth.plan_from_id_token() {
                            snapshot.plan = Some(PlanInfo {
                                name: Labeled::provider_local(plan_name),
                                tier: None,
                            });
                        }
                    }
                }
                return Ok(snapshot);
            }
            Err(e) => debug!("Codex CLI RPC failed: {}", e),
        }

        // Strategy 2: Fall back to local auth file for plan-only info
        if let Some(auth) = self.read_auth_file() {
            if let Some(plan_name) = auth.plan_from_id_token() {
                info!("CLI RPC failed, returning plan info from local JWT");
                return Ok(UsageSnapshot {
                    provider: ProviderId::Codex,
                    fetched_at: Utc::now(),
                    plan: Some(PlanInfo {
                        name: Labeled::provider_local(plan_name),
                        tier: None,
                    }),
                    buckets: vec![],
                    source_strategy: "local_jwt".into(),
                    notes: vec![
                        "Plan info from local auth token (no quota data available)".into(),
                        "Rate limit data requires Codex CLI app-server".into(),
                    ],
                });
            }
        }

        Err(CoreError::FetchFailed {
            provider: "codex".into(),
            reason: "all fetch strategies failed".into(),
        })
    }

    fn strategies(&self) -> Vec<&str> {
        vec!["cli_rpc", "local_jwt"]
    }
}

struct CodexAuthSession {
    provider: CodexProvider,
    state: AuthSessionState,
    status_text: String,
}

impl CodexAuthSession {
    fn new(http: Arc<reqwest::Client>) -> Self {
        Self {
            provider: CodexProvider::new(http),
            state: AuthSessionState::Ready,
            status_text: "Complete Codex CLI login externally, then confirm here.".into(),
        }
    }
}

#[async_trait]
impl ProviderAuthSession for CodexAuthSession {
    fn view(&self) -> AuthSessionView {
        AuthSessionView {
            provider: ProviderId::Codex,
            title: "Codex Authentication".into(),
            subtitle: Some("Uses your existing Codex CLI auth.json".into()),
            kind: AuthFlowKind::ExternalInstructions,
            verification_uri: None,
            user_code: None,
            status_text: self.status_text.clone(),
            help_text: vec![
                "Authenticate with the Codex CLI so ~/.codex/auth.json contains a usable token."
                    .into(),
                "Return here and press Enter to check credentials.".into(),
            ],
            can_cancel: true,
            can_confirm: true,
            confirm_label: Some(if self.state.is_terminal() {
                "Close".into()
            } else {
                "I've completed login".into()
            }),
            poll_interval_secs: None,
        }
    }

    async fn advance(
        &mut self,
        command: AuthSessionCommand,
    ) -> Result<AuthSessionState, CoreError> {
        match command {
            AuthSessionCommand::Start => {
                self.state = AuthSessionState::WaitingForUser;
                self.status_text = "Complete Codex CLI login externally, then confirm here.".into();
                Ok(self.state.clone())
            }
            AuthSessionCommand::Poll => Ok(self.state.clone()),
            AuthSessionCommand::Confirm => {
                let auth_state = self.provider.authenticate().await?;
                match auth_state {
                    AuthState::Authenticated | AuthState::Configured => {
                        self.status_text = "Authenticated successfully.".into();
                        self.state = AuthSessionState::Succeeded(auth_state);
                    }
                    AuthState::Failed(msg) => {
                        self.status_text = format!("Authentication failed: {}", msg);
                        self.state = AuthSessionState::Failed(msg);
                    }
                    AuthState::NotConfigured => {
                        let msg = "Still not configured. Complete Codex CLI login, then try again."
                            .to_string();
                        self.status_text = msg.clone();
                        self.state = AuthSessionState::Failed(msg);
                    }
                    AuthState::Expired => {
                        let msg = "Codex credentials are expired.".to_string();
                        self.status_text = msg.clone();
                        self.state = AuthSessionState::Failed(msg);
                    }
                }
                Ok(self.state.clone())
            }
            AuthSessionCommand::Cancel => {
                self.status_text = "Authentication cancelled.".into();
                self.state = AuthSessionState::Cancelled;
                Ok(self.state.clone())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC response types matching real `codex app-server` output
// ---------------------------------------------------------------------------

/// `account/read` → `result`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcAccountResult {
    account: Option<RpcAccount>,
    #[allow(dead_code)]
    requires_openai_auth: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcAccount {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    account_type: Option<String>,
    #[allow(dead_code)]
    email: Option<String>,
    plan_type: Option<String>,
}

/// `account/rateLimits/read` → `result`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcRateLimitsResult {
    rate_limits: Option<RpcRateLimitEntry>,
    /// Alternate: rate limits keyed by limit ID (e.g. "codex").
    /// We prefer the top-level `rateLimits` but this is here for completeness.
    #[allow(dead_code)]
    rate_limits_by_limit_id: Option<serde_json::Value>,
}

/// A single rate limit entry with primary/secondary windows and credits.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcRateLimitEntry {
    #[allow(dead_code)]
    limit_id: Option<String>,
    #[allow(dead_code)]
    limit_name: Option<serde_json::Value>,
    primary: Option<RpcRateLimitWindow>,
    secondary: Option<RpcRateLimitWindow>,
    #[allow(dead_code)]
    credits: Option<RpcCredits>,
    plan_type: Option<String>,
}

/// A single rate-limit window (primary or secondary).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcRateLimitWindow {
    used_percent: Option<f64>,
    window_duration_mins: Option<u64>,
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcCredits {
    #[allow(dead_code)]
    has_credits: Option<bool>,
    #[allow(dead_code)]
    unlimited: Option<bool>,
    #[allow(dead_code)]
    balance: Option<String>,
}

// ---------------------------------------------------------------------------
// Local auth file types (unchanged from original)
// ---------------------------------------------------------------------------

/// Matches the actual structure of `~/.codex/auth.json`.
#[derive(Debug, Deserialize)]
struct CodexAuth {
    #[allow(dead_code)]
    auth_mode: Option<String>,
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<CodexTokens>,
    #[allow(dead_code)]
    last_refresh: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexTokens {
    id_token: Option<String>,
    access_token: Option<String>,
    #[allow(dead_code)]
    refresh_token: Option<String>,
    #[allow(dead_code)]
    account_id: Option<String>,
}

impl CodexAuth {
    /// Get the best available access token.
    fn access_token(&self) -> Option<&str> {
        self.tokens
            .as_ref()
            .and_then(|t| t.access_token.as_deref())
            .or(self.openai_api_key.as_deref())
    }

    /// Try to extract plan info from the id_token JWT claims.
    fn plan_from_id_token(&self) -> Option<String> {
        let id_token = self.tokens.as_ref()?.id_token.as_ref()?;
        // JWT has 3 parts: header.payload.signature
        let parts: Vec<&str> = id_token.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        // Decode the payload (base64url)
        use base64::Engine;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload_bytes = engine.decode(parts[1]).ok()?;
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
        // The plan is at: https://api.openai.com/auth -> chatgpt_plan_type
        payload
            .get("https://api.openai.com/auth")
            .and_then(|auth| auth.get("chatgpt_plan_type"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    fn has_token(&self) -> bool {
        self.access_token().is_some()
    }
}
