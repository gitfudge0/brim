use std::sync::Arc;

use anyhow::Result;
use brim_core::models::ProviderId;
use brim_storage::config::AppConfig;
use brim_storage::paths::AppPaths;

pub fn run(paths: &AppPaths, config: &Arc<AppConfig>) -> Result<()> {
    println!("=== Brim Diagnostics ===");
    println!();

    // Paths
    println!("Paths:");
    println!("  Config dir:  {}", paths.config_dir.display());
    println!(
        "  Config file: {} {}",
        paths.config_file.display(),
        if paths.config_file.exists() {
            "(exists)"
        } else {
            "(missing)"
        }
    );
    println!("  State dir:   {}", paths.state_dir.display());
    println!(
        "  DB file:     {} {}",
        paths.db_file.display(),
        if paths.db_file.exists() {
            "(exists)"
        } else {
            "(missing)"
        }
    );
    println!();

    // Config summary
    println!("Config:");
    println!("  Poll interval: {}s", config.general.poll_interval_secs);
    println!("  Log level: {}", config.general.log_level);
    for id in ProviderId::all() {
        let pc = config.provider(*id);
        println!(
            "  {} - enabled: {}, poll: {}s",
            id.as_str(),
            pc.enabled,
            pc.poll_interval_secs
        );
    }
    println!();

    // Provider CLI availability
    println!("External tools:");
    for (name, cmd) in [("codex", "codex"), ("claude", "claude"), ("gh", "gh")] {
        let available = std::process::Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        println!(
            "  {}: {}",
            name,
            if available { "found" } else { "not found" }
        );
    }
    println!();

    // Local credential files
    println!("Local credentials:");
    let codex_auth = brim_auth::local_files::find_codex_auth_file();
    println!(
        "  Codex auth.json: {}",
        codex_auth
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into())
    );
    let claude_creds = brim_auth::local_files::find_claude_credentials_file();
    println!(
        "  Claude credentials: {}",
        claude_creds
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into())
    );
    println!();

    // Keyring check
    println!("Keyring:");
    match brim_storage::keyring_store::KeyringStore::get_secret(ProviderId::Copilot, "github_token")
    {
        Ok(Some(_)) => println!("  Copilot GitHub token: stored"),
        Ok(None) => println!("  Copilot GitHub token: not stored"),
        Err(e) => println!("  Copilot GitHub token: error ({})", e),
    }
    println!();

    Ok(())
}
