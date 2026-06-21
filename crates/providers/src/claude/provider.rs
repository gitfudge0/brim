use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
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

/// Claude provider implementation.
///
/// Strategy order:
/// 1. Local credentials file (~/.claude/.credentials.json) + OAuth usage API
/// 2. Keyring-stored sessionKey + claude.ai web API
pub struct ClaudeProvider {
    http: Arc<reqwest::Client>,
}

impl ClaudeProvider {
    pub fn new(http: Arc<reqwest::Client>) -> Self {
        Self { http }
    }

    fn local_subscription_plan(subscription_type: &str) -> PlanInfo {
        PlanInfo {
            name: Labeled::provider_local("Claude".to_string()),
            tier: Some(Labeled::provider_local(subscription_type.to_string())),
        }
    }

    fn enrich_plan_with_local_subscription(
        plan: &mut Option<PlanInfo>,
        subscription_type: Option<&str>,
    ) {
        let Some(subscription_type) = subscription_type else {
            return;
        };

        match plan {
            Some(plan) => {
                if plan.tier.is_none() {
                    plan.tier = Some(Labeled::provider_local(subscription_type.to_string()));
                }
            }
            None => {
                *plan = Some(Self::local_subscription_plan(subscription_type));
            }
        }
    }

    /// Try to read Claude CLI credentials.
    fn read_credentials(&self) -> Option<ClaudeCredentials> {
        let path = local_files::find_claude_credentials_file()?;
        debug!("Found Claude credentials at {}", path.display());
        match local_files::read_json_file::<ClaudeCredentialsFile>(&path) {
            Ok(file) => {
                let creds = file.into_credentials();
                if creds.oauth_access_token.is_some() {
                    debug!("Claude OAuth token found in credentials file");
                } else {
                    warn!("Claude credentials file found but no OAuth token present");
                }
                Some(creds)
            }
            Err(e) => {
                warn!("Failed to parse Claude credentials: {}", e);
                None
            }
        }
    }

    /// Fetch usage via the Anthropic OAuth usage endpoint.
    /// GET https://api.anthropic.com/api/oauth/usage
    async fn fetch_via_oauth(&self, token: &str) -> Result<UsageSnapshot, CoreError> {
        let resp = self
            .http
            .get("https://api.anthropic.com/api/oauth/usage")
            .header("Authorization", format!("Bearer {}", token))
            .header("anthropic-beta", "oauth-2025-04-20")
            .send()
            .await
            .map_err(|e| CoreError::FetchFailed {
                provider: "claude".into(),
                reason: e.to_string(),
            })?;

        if resp.status() == 429 {
            return Err(CoreError::RateLimited {
                provider: "claude".into(),
                retry_after_secs: resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok()),
            });
        }

        if resp.status() == 401 || resp.status() == 403 {
            return Err(CoreError::AuthFailed {
                provider: "claude".into(),
                reason: format!("HTTP {}", resp.status()),
            });
        }

        let body = resp.text().await.map_err(|e| CoreError::FetchFailed {
            provider: "claude".into(),
            reason: e.to_string(),
        })?;

        debug!(
            "Claude OAuth usage response: {}",
            &body[..body.len().min(500)]
        );

        self.parse_oauth_usage(&body)
    }

    fn parse_oauth_usage(&self, body: &str) -> Result<UsageSnapshot, CoreError> {
        let usage: ClaudeUsageResponse =
            serde_json::from_str(body).map_err(|e| CoreError::FetchFailed {
                provider: "claude".into(),
                reason: format!("parse error: {}", e),
            })?;

        let mut buckets = Vec::new();

        if let Some(ref w) = usage.five_hour {
            buckets.push(w.to_bucket(
                "five_hour",
                "Session (5h)",
                TimeWindow::session("5-hour session", 5 * 3600),
            ));
        }
        if let Some(ref w) = usage.seven_day {
            buckets.push(w.to_bucket(
                "seven_day",
                "Weekly (7d)",
                TimeWindow::weekly("7-day rolling"),
            ));
        }
        if let Some(ref w) = usage.seven_day_opus {
            buckets.push(w.to_bucket(
                "seven_day_opus",
                "Weekly Opus (7d)",
                TimeWindow::weekly("7-day Opus"),
            ));
        }
        if let Some(ref w) = usage.seven_day_sonnet {
            buckets.push(w.to_bucket(
                "seven_day_sonnet",
                "Weekly Sonnet (7d)",
                TimeWindow::weekly("7-day Sonnet"),
            ));
        }

        Ok(UsageSnapshot {
            provider: ProviderId::Claude,
            fetched_at: Utc::now(),
            plan: None,
            buckets,
            source_strategy: "oauth_usage".into(),
            notes: vec!["Data from Anthropic OAuth usage API (experimental)".into()],
        })
    }
}

#[async_trait]
impl Provider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn display_name(&self) -> &str {
        "Claude"
    }

    async fn auth_state(&self) -> AuthState {
        match self.read_credentials() {
            Some(creds) => {
                if creds.oauth_access_token.is_some() || creds.session_key.is_some() {
                    AuthState::Configured
                } else {
                    AuthState::NotConfigured
                }
            }
            None => AuthState::NotConfigured,
        }
    }

    async fn authenticate(&self) -> Result<AuthState, CoreError> {
        match self.read_credentials() {
            Some(creds) => {
                if let Some(ref token) = creds.oauth_access_token {
                    // Validate by making a test request
                    info!("Found Claude OAuth token, validating...");
                    match self.fetch_via_oauth(token).await {
                        Ok(_) => Ok(AuthState::Authenticated),
                        Err(CoreError::AuthFailed { .. }) => Ok(AuthState::Expired),
                        Err(e) => {
                            warn!("Claude auth validation failed: {}", e);
                            Ok(AuthState::Configured) // network issue, token might still be valid
                        }
                    }
                } else if creds.session_key.is_some() {
                    info!("Found Claude session key");
                    Ok(AuthState::Configured)
                } else {
                    Ok(AuthState::Failed(
                        "credentials file exists but has no usable token".into(),
                    ))
                }
            }
            None => {
                info!("No Claude credentials found. Run `claude` CLI to authenticate first.");
                Ok(AuthState::NotConfigured)
            }
        }
    }

    fn begin_auth_session(&self) -> Result<Box<dyn ProviderAuthSession>, CoreError> {
        Ok(Box::new(ClaudeAuthSession::new(self.http.clone())))
    }

    async fn fetch_usage(&self) -> Result<UsageSnapshot, CoreError> {
        let creds = self
            .read_credentials()
            .ok_or_else(|| CoreError::NotConfigured("claude".into()))?;

        // Strategy 1: OAuth token
        if let Some(ref token) = creds.oauth_access_token {
            match self.fetch_via_oauth(token).await {
                Ok(mut snapshot) => {
                    // Preserve OAuth plan data, but backfill tier from the local
                    // credentials file when the API omits it.
                    Self::enrich_plan_with_local_subscription(
                        &mut snapshot.plan,
                        creds.subscription_type.as_deref(),
                    );
                    return Ok(snapshot);
                }
                Err(e) => debug!("Claude OAuth fetch failed: {}", e),
            }

            // If OAuth API failed but we have credentials, return a minimal snapshot
            if let Some(ref sub_type) = creds.subscription_type {
                info!("OAuth API failed, returning plan info from local credentials");
                return Ok(UsageSnapshot {
                    provider: ProviderId::Claude,
                    fetched_at: Utc::now(),
                    plan: Some(Self::local_subscription_plan(sub_type)),
                    buckets: vec![],
                    source_strategy: "local_credentials".into(),
                    notes: vec![
                        "Plan info from local credentials (no quota data available)".into(),
                        "OAuth usage API may require specific scopes or may be unavailable".into(),
                    ],
                });
            }
        }

        // Strategy 2: Session key (web API) - future implementation
        if let Some(ref _session_key) = creds.session_key {
            debug!("Claude session key found but web API not yet implemented");
        }

        Err(CoreError::FetchFailed {
            provider: "claude".into(),
            reason: "all fetch strategies failed".into(),
        })
    }

    fn strategies(&self) -> Vec<&str> {
        vec!["cli_oauth", "session_key_web"]
    }
}

// --- Response types ---

/// Matches the actual structure of ~/.claude/.credentials.json:
/// ```json
/// {
///   "claudeAiOauth": {
///     "accessToken": "sk-ant-oat01-...",
///     "refreshToken": "sk-ant-ort01-...",
///     "expiresAt": 1757110734787,
///     "scopes": ["user:inference", "user:profile"],
///     "subscriptionType": "pro"
///   }
/// }
/// ```
#[derive(Debug, Deserialize)]
struct ClaudeCredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeOAuthData>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOAuthData {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[allow(dead_code)]
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    #[allow(dead_code)]
    #[serde(rename = "expiresAt")]
    expires_at: Option<u64>,
    #[allow(dead_code)]
    scopes: Option<Vec<String>>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
}

struct ClaudeAuthSession {
    provider: ClaudeProvider,
    state: AuthSessionState,
    status_text: String,
}

impl ClaudeAuthSession {
    fn new(http: Arc<reqwest::Client>) -> Self {
        Self {
            provider: ClaudeProvider::new(http),
            state: AuthSessionState::Ready,
            status_text: "Complete Claude login externally, then confirm here.".into(),
        }
    }
}

#[async_trait]
impl ProviderAuthSession for ClaudeAuthSession {
    fn view(&self) -> AuthSessionView {
        AuthSessionView {
            provider: ProviderId::Claude,
            title: "Claude Authentication".into(),
            subtitle: Some("Uses your existing Claude CLI / credentials file".into()),
            kind: AuthFlowKind::ExternalInstructions,
            verification_uri: None,
            user_code: None,
            status_text: self.status_text.clone(),
            help_text: vec![
                "Authenticate with the Claude CLI or ensure ~/.claude/.credentials.json is present.".into(),
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
                self.status_text = "Complete Claude login externally, then confirm here.".into();
                Ok(self.state.clone())
            }
            AuthSessionCommand::Poll => Ok(self.state.clone()),
            AuthSessionCommand::Confirm => {
                let auth_state = self.provider.authenticate().await?;
                match auth_state {
                    AuthState::Authenticated | AuthState::Configured => {
                        self.status_text =
                            "Authenticated successfully. Fresh data will appear on sync.".into();
                        self.state = AuthSessionState::Succeeded(auth_state);
                    }
                    AuthState::Expired => {
                        let msg =
                            "Credentials are present but expired. Re-authenticate in Claude CLI."
                                .to_string();
                        self.status_text = msg.clone();
                        self.state = AuthSessionState::Failed(msg);
                    }
                    AuthState::Failed(msg) => {
                        self.status_text = format!("Authentication failed: {}", msg);
                        self.state = AuthSessionState::Failed(msg);
                    }
                    AuthState::NotConfigured => {
                        let msg = "Still not configured. Complete Claude login, then try again."
                            .to_string();
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

/// Flattened credentials extracted from the file, used internally by the provider.
struct ClaudeCredentials {
    /// OAuth access token from claudeAiOauth.accessToken
    oauth_access_token: Option<String>,
    /// Subscription type (e.g. "pro")
    subscription_type: Option<String>,
    /// Session key (from manual import, not in the file by default)
    session_key: Option<String>,
}

impl ClaudeCredentialsFile {
    fn into_credentials(self) -> ClaudeCredentials {
        match self.claude_ai_oauth {
            Some(oauth) => ClaudeCredentials {
                oauth_access_token: oauth.access_token,
                subscription_type: oauth.subscription_type,
                session_key: None,
            },
            None => ClaudeCredentials {
                oauth_access_token: None,
                subscription_type: None,
                session_key: None,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeUsageResponse {
    five_hour: Option<ClaudeWindow>,
    seven_day: Option<ClaudeWindow>,
    seven_day_opus: Option<ClaudeWindow>,
    seven_day_sonnet: Option<ClaudeWindow>,
}

/// A usage window. The API reports `utilization` as percent *used* (0-100) and
/// optional dollar amounts; it does not report token counts or limits directly.
#[derive(Debug, Clone, Deserialize)]
struct ClaudeWindow {
    utilization: Option<f64>,
    used_dollars: Option<f64>,
    limit_dollars: Option<f64>,
    resets_at: Option<String>,
}

impl ClaudeWindow {
    fn to_bucket(&self, metric: &str, label: &str, mut window: TimeWindow) -> QuotaBucket {
        if let Some(ref reset) = self.resets_at {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset) {
                window = window.with_reset(dt.with_timezone(&Utc));
            }
        }
        QuotaBucket {
            metric: metric.into(),
            label: label.into(),
            window,
            used: self.used_dollars.map(Labeled::experimental),
            limit: self.limit_dollars.map(Labeled::experimental),
            percent_remaining: self
                .utilization
                .map(|u| Labeled::experimental((100.0 - u) / 100.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brim_core::confidence::Confidence;

    fn provider() -> ClaudeProvider {
        ClaudeProvider::new(Arc::new(reqwest::Client::new()))
    }

    #[test]
    fn parses_utilization_windows() {
        let snapshot = provider()
            .parse_oauth_usage(
                r#"{
                    "five_hour": {
                        "utilization": 3.0,
                        "resets_at": "2026-06-21T14:39:59.138466+00:00",
                        "used_dollars": null,
                        "limit_dollars": null
                    },
                    "seven_day": { "utilization": 6.0 },
                    "seven_day_sonnet": { "utilization": 2.0 }
                }"#,
            )
            .expect("oauth usage should parse");

        // plan is not in the usage API; left to local-credential enrichment
        assert!(snapshot.plan.is_none());

        let session = snapshot
            .buckets
            .iter()
            .find(|b| b.metric == "five_hour")
            .expect("session bucket present");
        // utilization 3% used -> 97% remaining (stored as fraction)
        let pct = session.percent_remaining.as_ref().expect("pct present");
        assert!((pct.value - 0.97).abs() < 1e-9);
        assert!(session.window.resets_at.is_some());

        assert!(snapshot.buckets.iter().any(|b| b.metric == "seven_day"));
        assert!(snapshot
            .buckets
            .iter()
            .any(|b| b.metric == "seven_day_sonnet"));
    }

    #[test]
    fn parses_empty_as_no_buckets() {
        let snapshot = provider()
            .parse_oauth_usage("{}")
            .expect("oauth usage should parse");

        assert!(snapshot.plan.is_none());
        assert!(snapshot.buckets.is_empty());
    }

    #[test]
    fn enriches_missing_oauth_tier_from_local_subscription() {
        let mut plan = Some(PlanInfo {
            name: Labeled::experimental("Claude".to_string()),
            tier: None,
        });

        ClaudeProvider::enrich_plan_with_local_subscription(&mut plan, Some("pro"));

        let plan = plan.expect("plan should still exist");
        assert_eq!(plan.name.value, "Claude");
        assert_eq!(plan.name.confidence, Confidence::Experimental);
        assert_eq!(plan.tier.as_ref().map(|t| t.value.as_str()), Some("pro"));
        assert_eq!(
            plan.tier.as_ref().map(|t| t.confidence),
            Some(Confidence::ProviderLocal)
        );
    }

    #[test]
    fn local_subscription_creates_claude_name_and_tier() {
        let plan = ClaudeProvider::local_subscription_plan("pro");

        assert_eq!(plan.name.value, "Claude");
        assert_eq!(plan.name.confidence, Confidence::ProviderLocal);
        assert_eq!(plan.tier.as_ref().map(|t| t.value.as_str()), Some("pro"));
        assert_eq!(
            plan.tier.as_ref().map(|t| t.confidence),
            Some(Confidence::ProviderLocal)
        );
    }

    #[test]
    fn local_enrichment_without_subscription_does_nothing() {
        let mut plan = None;

        ClaudeProvider::enrich_plan_with_local_subscription(&mut plan, None);

        assert!(plan.is_none());
    }
}
