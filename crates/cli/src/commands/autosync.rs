//! `brim autosync` — interval-based background syncing, supervised by the OS.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use brim_core::models::ProviderId;
use brim_providers::sync_engine::SyncEngine;
use brim_storage::config::AppConfig;
use brim_storage::paths::AppPaths;

use crate::service::{self, ServiceOutcome};

/// How often the run loop wakes to check whether any provider is due.
/// Sync cadence itself is per-provider `poll_interval_secs`; this is just the
/// granularity at which we notice config edits and due times.
const TICK_SECS: u64 = 5;

fn last_sync_key(id: ProviderId) -> String {
    format!("autosync:last_sync:{}", id.as_str())
}

/// Decides which providers are due to sync, given config and wall-clock time.
///
/// Kept separate from the loop and free of I/O so it can be unit-tested.
#[derive(Default)]
pub struct Scheduler {
    /// epoch-seconds of the last sync we kicked off, per provider.
    last_run: HashMap<ProviderId, i64>,
}

impl Scheduler {
    /// Enabled providers whose interval has elapsed (or that never ran).
    pub fn due(&self, config: &AppConfig, now_secs: i64) -> Vec<ProviderId> {
        config
            .enabled_provider_ids()
            .into_iter()
            .filter(|id| match self.last_run.get(id) {
                None => true,
                Some(last) => {
                    let elapsed = now_secs - last;
                    // elapsed < 0 means the clock jumped backward (NTP); re-sync
                    // rather than wedging until wall-clock catches up.
                    elapsed < 0 || elapsed >= config.poll_interval(*id) as i64
                }
            })
            .collect()
    }

    pub fn mark(&mut self, id: ProviderId, now_secs: i64) {
        self.last_run.insert(id, now_secs);
    }
}

/// The foreground loop the OS service supervises (`brim autosync run`).
///
/// Reloads config every tick so `autosync interval` / `provider enable` edits
/// apply live without restarting the service.
pub async fn run(engine: &SyncEngine, config_path: &Path) -> Result<()> {
    let mut scheduler = Scheduler::default();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(TICK_SECS));

    tracing::info!("autosync loop started (tick {}s)", TICK_SECS);

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let config = AppConfig::load(config_path).unwrap_or_default();
                let now = Utc::now().timestamp();
                for id in scheduler.due(&config, now) {
                    let result = engine.sync_provider(id).await;
                    scheduler.mark(id, now);
                    if result.failure.is_none() {
                        // Stamp only on success so `status` reflects real freshness.
                        let _ = engine
                            .db()
                            .set_meta(&last_sync_key(id), &Utc::now().to_rfc3339());
                    }
                }
            }
            _ = shutdown_signal() => {
                tracing::info!("autosync loop shutting down");
                break;
            }
        }
    }
    Ok(())
}

/// Resolve on SIGTERM (systemd stop) or SIGINT (Ctrl-C).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return std::future::pending().await,
        };
        let mut int = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(_) => return std::future::pending().await,
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// `brim autosync enable` — install + start the OS service.
pub fn enable() -> Result<()> {
    match service::install()? {
        ServiceOutcome::Done => {
            println!("Auto-sync enabled. brim will keep your usage data fresh in the background.");
        }
        ServiceOutcome::Unsupported => {
            println!(
                "No supported service manager (systemd/launchd) found.\n\
                 Run `brim autosync run` yourself, or wire it into your own supervisor."
            );
        }
    }
    Ok(())
}

/// `brim autosync disable` — stop + remove the OS service.
pub fn disable() -> Result<()> {
    match service::uninstall()? {
        ServiceOutcome::Done => println!("Auto-sync disabled."),
        ServiceOutcome::Unsupported => println!("No auto-sync service was installed."),
    }
    Ok(())
}

/// `brim autosync status` — is the service running, and when did each provider last sync?
pub fn status(engine: &SyncEngine) -> Result<()> {
    match service::is_active() {
        // is-active can't distinguish "stopped" from "not installed", so don't claim either.
        Some(true) => println!("Service: running"),
        Some(false) => println!("Service: not running (enable with `brim autosync enable`)"),
        None => println!("Service: not available (no systemd/launchd)"),
    }

    println!("Last sync:");
    for id in engine.config().enabled_provider_ids() {
        let when = engine
            .db()
            .get_meta(&last_sync_key(id))
            .ok()
            .flatten()
            .unwrap_or_else(|| "never".into());
        println!("  {:<8} {}", id.as_str(), when);
    }
    if engine.config().enabled_provider_ids().is_empty() {
        println!("  (no providers enabled)");
    }
    Ok(())
}

/// `brim autosync interval [secs] [--provider id]` — get or set the cadence.
pub fn interval(provider: Option<ProviderId>, secs: Option<u64>) -> Result<()> {
    let paths = AppPaths::resolve().map_err(|e| anyhow::anyhow!("{}", e))?;
    paths.ensure_dirs().map_err(|e| anyhow::anyhow!("{}", e))?;
    let mut config = AppConfig::load(&paths.config_file).map_err(|e| anyhow::anyhow!("{}", e))?;

    match secs {
        // Set
        Some(secs) => {
            if secs == 0 {
                anyhow::bail!("interval must be at least 1 second");
            }
            config.set_poll_interval(provider, secs);
            config
                .save(&paths.config_file)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            match provider {
                Some(id) => println!("{} sync interval set to {}s.", id.display_name(), secs),
                None => println!("Default sync interval set to {secs}s."),
            }
            println!("Running auto-sync picks this up within a few seconds; no restart needed.");
        }
        // Get
        None => {
            println!("Default: {}s", config.general.poll_interval_secs);
            for id in ProviderId::all() {
                println!("  {:<8} {}s", id.as_str(), config.poll_interval(*id));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_returns_enabled_providers_first_time() {
        let mut config = AppConfig::default();
        config.set_provider_enabled(ProviderId::Claude, true);
        config.set_provider_enabled(ProviderId::Codex, true);
        let scheduler = Scheduler::default();

        let due = scheduler.due(&config, 1000);
        assert!(due.contains(&ProviderId::Claude));
        assert!(due.contains(&ProviderId::Codex));
        // Disabled provider is never due.
        assert!(!due.contains(&ProviderId::Copilot));
    }

    #[test]
    fn due_respects_interval_after_mark() {
        let mut config = AppConfig::default();
        config.set_provider_enabled(ProviderId::Claude, true);
        config.set_poll_interval(Some(ProviderId::Claude), 300);

        let mut scheduler = Scheduler::default();
        scheduler.mark(ProviderId::Claude, 1000);

        // Too soon.
        assert!(!scheduler.due(&config, 1200).contains(&ProviderId::Claude));
        // Interval elapsed.
        assert!(scheduler.due(&config, 1300).contains(&ProviderId::Claude));
    }

    #[test]
    fn due_resyncs_when_clock_jumps_backward() {
        let mut config = AppConfig::default();
        config.set_provider_enabled(ProviderId::Claude, true);

        let mut scheduler = Scheduler::default();
        scheduler.mark(ProviderId::Claude, 5000);
        // Clock jumped back; don't wedge until wall-clock catches up.
        assert!(scheduler.due(&config, 1000).contains(&ProviderId::Claude));
    }
}
