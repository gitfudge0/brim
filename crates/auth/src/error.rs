use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("no credentials found for {provider}")]
    NoCredentials { provider: String },

    #[error("credential expired for {provider}")]
    Expired { provider: String },

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("device flow error: {0}")]
    DeviceFlow(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("{0}")]
    Other(String),
}
