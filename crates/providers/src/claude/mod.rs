//! Claude provider.
//!
//! Fetch strategies (in order of preference):
//! 1. Claude CLI OAuth token reuse → /api/oauth/usage
//! 2. Local credentials file (~/.claude/.credentials.json) + API
//! 3. Browser sessionKey cookie + claude.ai/api/organizations/{orgId}/usage (later)

mod provider;

pub use provider::ClaudeProvider;
