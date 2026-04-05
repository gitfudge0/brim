use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::{debug, info, warn};

use brim_core::confidence::Labeled;
use brim_core::error::CoreError;
use brim_core::models::{AuthState, PlanInfo, ProviderId, QuotaBucket, UsageSnapshot};
use brim_core::provider::{
    AuthFlowKind, AuthSessionCommand, AuthSessionState, AuthSessionView, Provider,
    ProviderAuthSession,
};
use brim_core::time_window::{TimeWindow, WindowKind};

use brim_auth::device_flow;
use brim_storage::keyring_store::KeyringStore;

/// GitHub Copilot provider implementation.
///
/// Strategy order:
/// 1. GitHub OAuth token (from keyring or device flow) → copilot_internal/user
/// 2. `gh auth token` CLI fallback
pub struct CopilotProvider {
    http: Arc<reqwest::Client>,
}

// GitHub OAuth App client ID for Copilot-like native apps.
// This is the VS Code Copilot extension's client ID, widely reused by similar tools.
const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_SCOPES: &str = "read:user";

const COPILOT_USER_URL: &str = "https://api.github.com/copilot_internal/user";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const EDITOR_VERSION: &str = "vscode/1.107.0";
const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";

impl CopilotProvider {
    pub fn new(http: Arc<reqwest::Client>) -> Self {
        Self { http }
    }

    /// Get stored GitHub token from keyring.
    fn get_stored_token(&self) -> Option<String> {
        match KeyringStore::get_secret(ProviderId::Copilot, "github_token") {
            Ok(Some(token)) => Some(token),
            Ok(None) => None,
            Err(e) => {
                warn!("Failed to read copilot token from keyring: {}", e);
                None
            }
        }
    }

    /// Try to get a token from the `gh` CLI.
    async fn get_gh_cli_token(&self) -> Option<String> {
        let output = tokio::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
            .await
            .ok()?;

        if output.status.success() {
            let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !token.is_empty() {
                debug!("Got token from gh CLI");
                return Some(token);
            }
        }
        None
    }

    /// Get the best available token (keyring first, then gh CLI).
    async fn resolve_token(&self) -> Option<String> {
        if let Some(token) = self.get_stored_token() {
            return Some(token);
        }
        self.get_gh_cli_token().await
    }

    fn copilot_headers(&self) -> [(&'static str, &'static str); 5] {
        [
            ("User-Agent", USER_AGENT),
            ("Editor-Version", EDITOR_VERSION),
            ("Editor-Plugin-Version", EDITOR_PLUGIN_VERSION),
            ("Copilot-Integration-Id", "vscode-chat"),
            ("Accept", "application/json"),
        ]
    }

    async fn fetch_copilot_usage_with_token(
        &self,
        token: &str,
        auth_scheme: &str,
        source_strategy: &str,
    ) -> Result<UsageSnapshot, CoreError> {
        let mut req = self
            .http
            .get(COPILOT_USER_URL)
            .header("Authorization", format!("{} {}", auth_scheme, token));

        for (name, value) in self.copilot_headers() {
            req = req.header(name, value);
        }

        let resp = req.send().await.map_err(|e| CoreError::FetchFailed {
            provider: "copilot".into(),
            reason: e.to_string(),
        })?;

        if resp.status() == 401 || resp.status() == 403 {
            return Err(CoreError::AuthFailed {
                provider: "copilot".into(),
                reason: format!("HTTP {} - token may lack copilot scope", resp.status()),
            });
        }

        if resp.status() == 429 {
            return Err(CoreError::RateLimited {
                provider: "copilot".into(),
                retry_after_secs: resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok()),
            });
        }

        let body = resp.text().await.map_err(|e| CoreError::FetchFailed {
            provider: "copilot".into(),
            reason: e.to_string(),
        })?;

        debug!("Copilot user response: {}", &body[..body.len().min(500)]);

        self.parse_copilot_user(&body, source_strategy)
    }

    async fn exchange_for_copilot_token(&self, token: &str) -> Option<String> {
        let mut req = self
            .http
            .get(COPILOT_TOKEN_URL)
            .header("Authorization", format!("Bearer {}", token));

        for (name, value) in self.copilot_headers() {
            req = req.header(name, value);
        }

        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            debug!("Copilot token exchange failed with HTTP {}", resp.status());
            return None;
        }

        let body: CopilotTokenResponse = resp.json().await.ok()?;
        body.token
    }

    /// Fetch usage from the copilot_internal/user endpoint.
    async fn fetch_copilot_usage(&self, token: &str) -> Result<UsageSnapshot, CoreError> {
        match self
            .fetch_copilot_usage_with_token(token, "token", "copilot_internal_user")
            .await
        {
            Ok(snapshot) => Ok(snapshot),
            Err(CoreError::AuthFailed { .. }) => {
                if let Some(exchanged) = self.exchange_for_copilot_token(token).await {
                    return self
                        .fetch_copilot_usage_with_token(
                            &exchanged,
                            "Bearer",
                            "copilot_internal_user_exchanged",
                        )
                        .await;
                }
                Err(CoreError::AuthFailed {
                    provider: "copilot".into(),
                    reason: "token rejected by Copilot internal API and token exchange failed"
                        .into(),
                })
            }
            Err(e) => Err(e),
        }
    }

    fn parse_copilot_user(
        &self,
        body: &str,
        source_strategy: &str,
    ) -> Result<UsageSnapshot, CoreError> {
        let user: CopilotUserResponse =
            serde_json::from_str(body).map_err(|e| CoreError::FetchFailed {
                provider: "copilot".into(),
                reason: format!("parse error: {}", e),
            })?;

        let mut buckets = Vec::new();
        let reset_at = user
            .quota_reset_date_utc
            .as_deref()
            .or(user.quota_reset_date.as_deref())
            .and_then(parse_datetime);

        if let Some(ref snapshots) = user.quota_snapshots {
            if let Some(ref premium) = snapshots.premium_interactions {
                if let Some(bucket) = quota_bucket_from_snapshot(
                    "premium_interactions",
                    "Premium Requests",
                    premium,
                    reset_at,
                ) {
                    buckets.push(bucket);
                }
            }

            if let Some(ref chat) = snapshots.chat {
                if let Some(bucket) = quota_bucket_from_snapshot("chat", "Chat", chat, reset_at) {
                    buckets.push(bucket);
                }
            }

            if let Some(ref completions) = snapshots.completions {
                if let Some(bucket) =
                    quota_bucket_from_snapshot("completions", "Completions", completions, reset_at)
                {
                    buckets.push(bucket);
                }
            }
        }

        let plan = user.copilot_plan.as_ref().map(|p| PlanInfo {
            name: Labeled::experimental(p.clone()),
            tier: None,
        });

        Ok(UsageSnapshot {
            provider: ProviderId::Copilot,
            fetched_at: Utc::now(),
            plan,
            buckets,
            source_strategy: source_strategy.into(),
            notes: vec!["Data from internal GitHub Copilot API (experimental)".into()],
        })
    }

    /// Run the GitHub device flow interactively.
    async fn run_device_flow(&self) -> Result<String, CoreError> {
        let device_code =
            device_flow::request_device_code(&self.http, GITHUB_CLIENT_ID, GITHUB_SCOPES)
                .await
                .map_err(|e| CoreError::AuthFailed {
                    provider: "copilot".into(),
                    reason: e.to_string(),
                })?;

        println!();
        println!("  GitHub Device Authorization");
        println!("  ---------------------------");
        println!("  Open: {}", device_code.verification_uri);
        println!("  Enter code: {}", device_code.user_code);
        println!();
        println!("  Waiting for authorization...");

        let interval = std::time::Duration::from_secs(device_code.interval.max(5));
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(device_code.expires_in);

        loop {
            tokio::time::sleep(interval).await;

            if std::time::Instant::now() > deadline {
                return Err(CoreError::AuthFailed {
                    provider: "copilot".into(),
                    reason: "device code expired".into(),
                });
            }

            let resp =
                device_flow::poll_for_token(&self.http, GITHUB_CLIENT_ID, &device_code.device_code)
                    .await
                    .map_err(|e| CoreError::AuthFailed {
                        provider: "copilot".into(),
                        reason: e.to_string(),
                    })?;

            match device_flow::PollResult::from(resp) {
                device_flow::PollResult::Success { access_token } => {
                    // Store in keyring
                    KeyringStore::set_secret(ProviderId::Copilot, "github_token", &access_token)
                        .map_err(|e| CoreError::AuthFailed {
                            provider: "copilot".into(),
                            reason: format!("failed to store token: {}", e),
                        })?;
                    println!("  Authenticated successfully!");
                    return Ok(access_token);
                }
                device_flow::PollResult::Pending => {
                    continue;
                }
                device_flow::PollResult::Expired => {
                    return Err(CoreError::AuthFailed {
                        provider: "copilot".into(),
                        reason: "device code expired".into(),
                    });
                }
                device_flow::PollResult::Error(msg) => {
                    return Err(CoreError::AuthFailed {
                        provider: "copilot".into(),
                        reason: msg,
                    });
                }
            }
        }
    }
}

#[async_trait]
impl Provider for CopilotProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Copilot
    }

    fn display_name(&self) -> &str {
        "GitHub Copilot"
    }

    async fn auth_state(&self) -> AuthState {
        if self.get_stored_token().is_some() || self.get_gh_cli_token().await.is_some() {
            AuthState::Configured
        } else {
            AuthState::NotConfigured
        }
    }

    async fn authenticate(&self) -> Result<AuthState, CoreError> {
        // Check for existing token first
        if let Some(token) = self.resolve_token().await {
            // Validate it
            match self.fetch_copilot_usage(&token).await {
                Ok(_) => {
                    // Store if it came from gh CLI (not already in keyring)
                    if self.get_stored_token().is_none() {
                        let _ =
                            KeyringStore::set_secret(ProviderId::Copilot, "github_token", &token);
                    }
                    return Ok(AuthState::Authenticated);
                }
                Err(CoreError::AuthFailed { .. }) => {
                    info!("Existing token is invalid, starting device flow");
                }
                Err(e) => {
                    warn!("Token validation failed (network?): {}", e);
                    return Ok(AuthState::Configured); // might work later
                }
            }
        }

        // Run device flow
        self.run_device_flow().await?;
        Ok(AuthState::Authenticated)
    }

    fn begin_auth_session(&self) -> Result<Box<dyn ProviderAuthSession>, CoreError> {
        Ok(Box::new(CopilotAuthSession::new(self.http.clone())))
    }

    async fn fetch_usage(&self) -> Result<UsageSnapshot, CoreError> {
        let token = self
            .resolve_token()
            .await
            .ok_or_else(|| CoreError::NotConfigured("copilot".into()))?;

        self.fetch_copilot_usage(&token).await
    }

    fn strategies(&self) -> Vec<&str> {
        vec![
            "copilot_internal_user",
            "copilot_internal_user_exchanged",
            "gh_cli_token",
        ]
    }
}

fn quota_bucket_from_snapshot(
    metric: &str,
    label: &str,
    quota: &CopilotQuota,
    reset_at: Option<DateTime<Utc>>,
) -> Option<QuotaBucket> {
    if quota.unlimited.unwrap_or(false) {
        return None;
    }

    let limit = quota.entitlement.or(quota.limit)?;
    if limit <= 0.0 {
        return None;
    }

    let used = quota
        .used
        .or_else(|| match (quota.entitlement, quota.remaining) {
            (Some(entitlement), Some(remaining)) => Some((entitlement - remaining).max(0.0)),
            _ => None,
        })
        .unwrap_or(0.0);

    let percent_remaining = quota
        .percentage_remaining
        .map(|p| (p / 100.0).clamp(0.0, 1.0))
        .or_else(|| {
            quota
                .remaining
                .map(|remaining| (remaining / limit).clamp(0.0, 1.0))
        });

    let mut window = TimeWindow {
        kind: WindowKind::Monthly,
        label: "Monthly cycle".into(),
        duration_secs: None,
        resets_at: None,
    };
    if let Some(reset_at) = reset_at {
        window = window.with_reset(reset_at);
    }

    Some(QuotaBucket {
        metric: metric.into(),
        label: label.into(),
        window,
        used: Some(Labeled::experimental(used)),
        limit: Some(Labeled::experimental(limit)),
        percent_remaining: percent_remaining.map(Labeled::experimental),
    })
}

fn parse_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

// --- Response types ---

#[derive(Debug, Deserialize)]
struct CopilotUserResponse {
    #[serde(alias = "copilotPlan", alias = "copilot_plan")]
    copilot_plan: Option<String>,
    #[serde(alias = "quotaResetDate", alias = "quota_reset_date")]
    quota_reset_date: Option<String>,
    #[serde(alias = "quotaResetDateUTC", alias = "quota_reset_date_utc")]
    quota_reset_date_utc: Option<String>,
    #[serde(alias = "quotaSnapshots", alias = "quota_snapshots")]
    quota_snapshots: Option<CopilotQuotaSnapshots>,
}

#[derive(Debug, Deserialize)]
struct CopilotQuotaSnapshots {
    #[serde(alias = "premiumInteractions", alias = "premium_interactions")]
    premium_interactions: Option<CopilotQuota>,
    chat: Option<CopilotQuota>,
    completions: Option<CopilotQuota>,
}

#[derive(Debug, Deserialize)]
struct CopilotQuota {
    entitlement: Option<f64>,
    remaining: Option<f64>,
    unlimited: Option<bool>,
    used: Option<f64>,
    limit: Option<f64>,
    #[serde(alias = "percentageRemaining", alias = "percentage_remaining")]
    percentage_remaining: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: Option<String>,
}

struct CopilotAuthSession {
    provider: CopilotProvider,
    device_code: Option<device_flow::DeviceCodeResponse>,
    state: AuthSessionState,
    status_text: String,
}

impl CopilotAuthSession {
    fn new(http: Arc<reqwest::Client>) -> Self {
        Self {
            provider: CopilotProvider::new(http),
            device_code: None,
            state: AuthSessionState::Ready,
            status_text: "Starting GitHub device authorization...".into(),
        }
    }
}

#[async_trait]
impl ProviderAuthSession for CopilotAuthSession {
    fn view(&self) -> AuthSessionView {
        AuthSessionView {
            provider: ProviderId::Copilot,
            title: "GitHub Device Authorization".into(),
            subtitle: Some("Authenticate GitHub Copilot inside the tracker".into()),
            kind: AuthFlowKind::DeviceCode,
            verification_uri: self
                .device_code
                .as_ref()
                .map(|device_code| device_code.verification_uri.clone()),
            user_code: self
                .device_code
                .as_ref()
                .map(|device_code| device_code.user_code.clone()),
            status_text: self.status_text.clone(),
            help_text: vec![
                "Open the URL in your browser and enter the device code.".into(),
                "Leave this popup open while authorization completes.".into(),
            ],
            can_cancel: !self.state.is_terminal(),
            can_confirm: self.state.is_terminal(),
            confirm_label: self.state.is_terminal().then(|| "Close".to_string()),
            poll_interval_secs: self
                .device_code
                .as_ref()
                .map(|device_code| device_code.interval.max(5)),
        }
    }

    async fn advance(
        &mut self,
        command: AuthSessionCommand,
    ) -> Result<AuthSessionState, CoreError> {
        match command {
            AuthSessionCommand::Start => {
                let device_code = device_flow::request_device_code(
                    &self.provider.http,
                    GITHUB_CLIENT_ID,
                    GITHUB_SCOPES,
                )
                .await
                .map_err(|e| CoreError::AuthFailed {
                    provider: "copilot".into(),
                    reason: e.to_string(),
                })?;

                self.status_text = "Waiting for authorization...".into();
                self.device_code = Some(device_code);
                self.state = AuthSessionState::WaitingForUser;
                Ok(self.state.clone())
            }
            AuthSessionCommand::Poll => {
                let device_code = self.device_code.as_ref().ok_or_else(|| {
                    CoreError::Other("device code session not initialized".into())
                })?;
                self.state = AuthSessionState::Polling;
                self.status_text = "Polling GitHub...".into();

                let resp = device_flow::poll_for_token(
                    &self.provider.http,
                    GITHUB_CLIENT_ID,
                    &device_code.device_code,
                )
                .await
                .map_err(|e| CoreError::AuthFailed {
                    provider: "copilot".into(),
                    reason: e.to_string(),
                })?;

                match device_flow::PollResult::from(resp) {
                    device_flow::PollResult::Success { access_token } => {
                        KeyringStore::set_secret(
                            ProviderId::Copilot,
                            "github_token",
                            &access_token,
                        )
                        .map_err(|e| CoreError::AuthFailed {
                            provider: "copilot".into(),
                            reason: format!("failed to store token: {}", e),
                        })?;
                        self.status_text = "Authenticated successfully.".into();
                        self.state = AuthSessionState::Succeeded(AuthState::Authenticated);
                        Ok(self.state.clone())
                    }
                    device_flow::PollResult::Pending => {
                        self.status_text = "Waiting for authorization...".into();
                        self.state = AuthSessionState::WaitingForUser;
                        Ok(self.state.clone())
                    }
                    device_flow::PollResult::Expired => {
                        let msg = "Device code expired.".to_string();
                        self.status_text = msg.clone();
                        self.state = AuthSessionState::Failed(msg);
                        Ok(self.state.clone())
                    }
                    device_flow::PollResult::Error(msg) => {
                        self.status_text = format!("Authorization failed: {}", msg);
                        self.state = AuthSessionState::Failed(msg);
                        Ok(self.state.clone())
                    }
                }
            }
            AuthSessionCommand::Confirm => Ok(self.state.clone()),
            AuthSessionCommand::Cancel => {
                self.status_text = "Authorization cancelled.".into();
                self.state = AuthSessionState::Cancelled;
                Ok(self.state.clone())
            }
        }
    }
}
