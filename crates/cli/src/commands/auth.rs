use anyhow::Result;
use brim_core::models::ProviderId;
use brim_providers::sync_engine::SyncEngine;

pub async fn status(engine: &SyncEngine) -> Result<()> {
    for provider in engine.registry().all() {
        let state = provider.auth_state().await;
        let strategies = provider.strategies().join(", ");
        println!(
            "{}: {} (strategies: {})",
            provider.display_name(),
            state,
            strategies
        );
    }
    Ok(())
}

pub async fn login(engine: &SyncEngine, provider_name: &str) -> Result<()> {
    let id: ProviderId = provider_name
        .parse()
        .map_err(|e: brim_core::error::CoreError| anyhow::anyhow!("{}", e))?;
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
    let id: ProviderId = provider_name
        .parse()
        .map_err(|e: brim_core::error::CoreError| anyhow::anyhow!("{}", e))?;

    // Clear keyring secrets
    match brim_storage::keyring_store::KeyringStore::delete_secret(id, "github_token") {
        Ok(()) => {}
        Err(e) => eprintln!("Warning: failed to clear keyring: {}", e),
    }

    println!("Cleared credentials for {}", id.display_name());
    Ok(())
}
