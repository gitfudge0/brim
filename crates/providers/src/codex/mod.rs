//! Codex / ChatGPT provider.
//!
//! Fetch strategies (in order of preference):
//! 1. Codex CLI JSON-RPC (`codex app-server` → `account/read`, `account/rateLimits/read`)
//! 2. Local auth file (~/.codex/auth.json) + internal API
//! 3. Browser cookie import + web scraping (later milestone)

mod provider;

pub use provider::CodexProvider;
