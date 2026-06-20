use anyhow::Result;
use brim_providers::sync_engine::SyncEngine;

pub async fn run(engine: &SyncEngine, provider: Option<String>) -> Result<()> {
    let ids = match provider {
        Some(name) => vec![crate::commands::parse_provider_arg(&name)?],
        None => engine.config().enabled_provider_ids(),
    };

    if ids.is_empty() {
        anyhow::bail!("{}", crate::commands::no_enabled_providers_message());
    }

    let mut success_count = 0usize;
    let mut failure_count = 0usize;

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
                success_count += 1;
            }
            None => {
                let err = result
                    .failure
                    .map(|failure| failure.message)
                    .unwrap_or_else(|| "unknown error".into());
                println!("{}: FAILED - {}", id.display_name(), err);
                failure_count += 1;
            }
        }
    }

    let pruned = engine.prune_old_data(engine.config().general.prune_after_days as i64);
    println!(
        "Summary: {} provider(s) synced successfully, {} failed.",
        success_count, failure_count
    );
    if pruned > 0 {
        println!("Pruned {} old snapshot(s).", pruned);
    }

    if failure_count > 0 {
        anyhow::bail!("one or more providers failed to sync");
    }

    Ok(())
}
