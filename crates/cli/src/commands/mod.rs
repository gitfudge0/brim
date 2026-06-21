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

/// Print the ASCII brim mark + wordmark. Only call on a TTY (it's decorative).
pub fn print_banner() {
    use brim_core::brand::{MARK, NAME, TAGLINE};
    // mark on the left, wordmark stacked on the right of the middle rows
    println!("\x1b[32m{}\x1b[0m", MARK[0]);
    println!("\x1b[32m{}\x1b[0m  \x1b[1;36m{}\x1b[0m", MARK[1], NAME);
    println!(
        "\x1b[32m{}\x1b[0m  \x1b[90m{}\x1b[0m",
        MARK[2], TAGLINE
    );
    println!();
}

pub fn no_enabled_providers_message() -> &'static str {
    "No providers are enabled. Run 'brim config init', enable a provider in the config, then run 'brim auth login <provider>'."
}
