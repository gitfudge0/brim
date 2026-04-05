use serde::{Deserialize, Serialize};

use crate::error::AuthError;

/// GitHub OAuth device flow implementation.
///
/// Used by the Copilot provider. Flow:
/// 1. POST /login/device/code → get device_code, user_code, verification_uri
/// 2. User opens verification_uri and enters user_code
/// 3. Poll POST /login/oauth/access_token until authorized
///
/// Reference: https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Request a device code from GitHub.
pub async fn request_device_code(
    client: &reqwest::Client,
    client_id: &str,
    scope: &str,
) -> Result<DeviceCodeResponse, AuthError> {
    let resp = client
        .post(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&[("client_id", client_id), ("scope", scope)])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| AuthError::DeviceFlow(e.to_string()))?;

    let body: DeviceCodeResponse = resp.json().await?;
    Ok(body)
}

/// Poll for an access token (call this in a loop with the specified interval).
pub async fn poll_for_token(
    client: &reqwest::Client,
    client_id: &str,
    device_code: &str,
) -> Result<TokenResponse, AuthError> {
    let resp = client
        .post(GITHUB_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| AuthError::DeviceFlow(e.to_string()))?;

    let body: TokenResponse = resp.json().await?;
    Ok(body)
}

/// Possible states when polling for token.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PollResult {
    /// Got an access token successfully.
    Success { access_token: String },
    /// User hasn't authorized yet, keep polling.
    Pending,
    /// The device code has expired.
    Expired,
    /// Some other error.
    Error(String),
}

impl From<TokenResponse> for PollResult {
    fn from(resp: TokenResponse) -> Self {
        if let Some(token) = resp.access_token {
            return PollResult::Success {
                access_token: token,
            };
        }
        match resp.error.as_deref() {
            Some("authorization_pending") => PollResult::Pending,
            Some("slow_down") => PollResult::Pending, // treat as pending, caller should increase interval
            Some("expired_token") => PollResult::Expired,
            Some(err) => {
                PollResult::Error(resp.error_description.unwrap_or_else(|| err.to_string()))
            }
            None => PollResult::Error("unexpected empty response".into()),
        }
    }
}
