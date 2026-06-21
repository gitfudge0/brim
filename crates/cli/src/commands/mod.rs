pub mod auth;
pub mod autosync;
pub mod config;
pub mod diag;
pub mod history;
pub mod json;
pub mod provider;
pub mod status;
pub mod sync;
pub mod uninstall;

use anyhow::{anyhow, Result};
use brim_core::models::ProviderId;

pub fn parse_provider_arg(name: &str) -> Result<ProviderId> {
    name.parse().map_err(|_| {
        anyhow!(
            "Unknown provider '{}'. Expected one of: codex, claude, copilot. Aliases: chatgpt/openai, anthropic, github.",
            name
        )
    })
}

pub fn no_enabled_providers_message() -> &'static str {
    "No providers are enabled. Run 'brim config init', enable a provider in the config, then run 'brim auth login <provider>'."
}
