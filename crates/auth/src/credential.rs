use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A generic credential container that can hold tokens from any provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    /// The access/bearer token.
    pub access_token: String,
    /// Optional refresh token.
    pub refresh_token: Option<String>,
    /// When the access token expires, if known.
    pub expires_at: Option<DateTime<Utc>>,
    /// The type of token (e.g. "Bearer", "session_key").
    pub token_type: String,
    /// Where this credential came from (e.g. "codex_auth_json", "device_flow", "cookie_import").
    pub source: String,
}

impl Credential {
    /// Check if the token is expired (or will expire within the given buffer).
    pub fn is_expired(&self, buffer_secs: i64) -> bool {
        match self.expires_at {
            Some(exp) => {
                let now = Utc::now();
                let buffer = chrono::Duration::seconds(buffer_secs);
                exp - buffer <= now
            }
            None => false, // no expiry info, assume valid
        }
    }

    /// Check if the token is still valid (not expired, with 60s buffer).
    pub fn is_valid(&self) -> bool {
        !self.is_expired(60)
    }
}
