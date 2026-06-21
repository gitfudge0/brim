use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

use brim_storage::config::AppConfig;
use brim_storage::db::Database;
use brim_storage::paths::AppPaths;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::sink)
        .init();

    let paths = AppPaths::resolve().map_err(|e| anyhow::anyhow!("{}", e))?;
    paths.ensure_dirs().map_err(|e| anyhow::anyhow!("{}", e))?;
    let config = AppConfig::load(&paths.config_file).map_err(|e| anyhow::anyhow!("{}", e))?;
    let db = Database::open(&paths.db_file).map_err(|e| anyhow::anyhow!("database: {}", e))?;
    let http = reqwest::Client::builder()
        .user_agent("brim/0.1")
        .timeout(Duration::from_secs(30))
        .build()?;

    brim_tui::run(paths, Arc::new(config), Arc::new(db), Arc::new(http))
}
