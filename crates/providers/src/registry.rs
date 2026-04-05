use std::sync::Arc;

use brim_core::provider::ProviderRegistry;
use brim_storage::config::AppConfig;

use crate::claude::ClaudeProvider;
use crate::codex::CodexProvider;
use crate::copilot::CopilotProvider;

/// Build the default provider registry with all supported providers.
pub fn build_registry(http: Arc<reqwest::Client>, config: &AppConfig) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();

    if config
        .provider(brim_core::models::ProviderId::Codex)
        .enabled
    {
        registry.register(Box::new(CodexProvider::new(http.clone())));
    }
    if config
        .provider(brim_core::models::ProviderId::Claude)
        .enabled
    {
        registry.register(Box::new(ClaudeProvider::new(http.clone())));
    }
    if config
        .provider(brim_core::models::ProviderId::Copilot)
        .enabled
    {
        registry.register(Box::new(CopilotProvider::new(http.clone())));
    }

    registry
}
