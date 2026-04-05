use anyhow::Result;
use brim_core::models::ProviderId;
use brim_providers::sync_engine::SyncEngine;

pub async fn run(engine: &SyncEngine, provider: Option<String>) -> Result<()> {
    let ids = match provider {
        Some(name) => {
            let id: ProviderId = name
                .parse()
                .map_err(|e: brim_core::error::CoreError| anyhow::anyhow!("{}", e))?;
            vec![id]
        }
        None => engine.registry().ids(),
    };

    for id in &ids {
        let result = engine.sync_provider(*id).await;
        match result.snapshot {
            Some(snap) => {
                println!(
                    "{}: OK - {} buckets via {} at {}",
                    id.display_name(),
                    snap.buckets.len(),
                    snap.source_strategy,
                    snap.fetched_at.format("%H:%M:%S"),
                );
            }
            None => {
                let err = result
                    .failure
                    .map(|failure| failure.message)
                    .unwrap_or_else(|| "unknown error".into());
                println!("{}: FAILED - {}", id.display_name(), err);
            }
        }
    }

    // Prune old data (keep 30 days)
    engine.prune_old_data(30);

    Ok(())
}
