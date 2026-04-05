//! GitHub Copilot provider.
//!
//! Fetch strategies (in order of preference):
//! 1. GitHub OAuth device flow → api.github.com/copilot_internal/user
//! 2. Existing GitHub CLI token reuse (gh auth token)

mod provider;

pub use provider::CopilotProvider;
