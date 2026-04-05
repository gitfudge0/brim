use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("unknown provider: {0}")]
    UnknownProvider(String),

    #[error("provider {0} is not configured")]
    NotConfigured(String),

    #[error("authentication failed for {provider}: {reason}")]
    AuthFailed { provider: String, reason: String },

    #[error("fetch failed for {provider}: {reason}")]
    FetchFailed { provider: String, reason: String },

    #[error("rate limited by {provider}, retry after {retry_after_secs:?}s")]
    RateLimited {
        provider: String,
        retry_after_secs: Option<u64>,
    },

    #[error("data is stale (last fetched {age_secs}s ago, TTL is {ttl_secs}s)")]
    StaleData { age_secs: u64, ttl_secs: u64 },

    #[error("{0}")]
    Other(String),
}
