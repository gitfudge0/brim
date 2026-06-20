use anyhow::Result;
use brim_core::models::ProviderId;
use brim_providers::sync_engine::SyncEngine;

pub async fn status(engine: &SyncEngine) -> Result<()> {
    for id in ProviderId::all() {
        let enabled = engine.config().provider(*id).enabled;
        let provider = engine
            .registry()
            .get(*id)
            .expect("supported providers should always exist in registry");
        let state = provider.auth_state().await;
        let strategies = provider.strategies().join(", ");
        println!(
            "{}: {} (enabled: {}, strategies: {})",
            provider.display_name(),
            state,
            if enabled { "yes" } else { "no" },
            strategies
        );
    }
    Ok(())
}

pub async fn login(engine: &SyncEngine, provider_name: &str) -> Result<()> {
    let id = crate::commands::parse_provider_arg(provider_name)?;
    let provider = engine
        .registry()
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("provider '{}' not found", provider_name))?;

    println!("Authenticating {}...", provider.display_name());
    let state = provider
        .authenticate()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Result: {}", state);
    Ok(())
}

pub async fn logout(provider_name: &str) -> Result<()> {
    let id = crate::commands::parse_provider_arg(provider_name)?;

    match id {
        ProviderId::Copilot => {
            match brim_storage::keyring_store::KeyringStore::delete_secret(id, "github_token") {
                Ok(()) => {
                    println!(
                        "Removed brim-managed GitHub token for {}",
                        id.display_name()
                    );
                }
                Err(e) => eprintln!("Warning: failed to clear keyring: {}", e),
            }
        }
        ProviderId::Codex => {
            println!(
                "brim does not manage Codex CLI auth directly. Remove ~/.codex/auth.json manually to log out globally."
            );
        }
        ProviderId::Claude => {
            println!(
                "brim does not manage Claude CLI credentials directly. Remove ~/.claude/.credentials.json manually to log out globally."
            );
        }
    }
    Ok(())
}
