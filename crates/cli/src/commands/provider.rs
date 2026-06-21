use anyhow::Result;
use brim_core::models::ProviderId;
use brim_storage::config::AppConfig;
use brim_storage::paths::AppPaths;

pub fn list(config: &AppConfig) {
    for id in ProviderId::all() {
        let enabled = config.provider(*id).enabled;
        println!(
            "  {}  {}",
            id.as_str(),
            if enabled { "[enabled]" } else { "[disabled]" }
        );
    }
}

pub fn set_enabled(id: ProviderId, enabled: bool) -> Result<()> {
    let paths = AppPaths::resolve().map_err(|e| anyhow::anyhow!("{}", e))?;
    paths.ensure_dirs().map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut config =
        AppConfig::load(&paths.config_file).map_err(|e| anyhow::anyhow!("{}", e))?;
    config.set_provider_enabled(id, enabled);
    config
        .save(&paths.config_file)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    println!(
        "{} {}.",
        id.display_name(),
        if enabled { "enabled" } else { "disabled" }
    );
    if enabled {
        println!("Run `brim auth login {}` if you haven't authenticated yet.", id.as_str());
    }
    Ok(())
}
