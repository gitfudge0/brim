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
    let mut missing_tools = Vec::new();
    for (name, cmd) in [("codex", "codex"), ("claude", "claude"), ("gh", "gh")] {
        let available = command_in_path(cmd);
        println!(
            "  {}: {}",
            name,
            if available { "found" } else { "not found" }
        );
        if !available {
            missing_tools.push(name);
        }
    }
    println!();

    // Local credential files
    println!("Local credential discovery:");
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

    println!("Summary:");
    let enabled = config.enabled_provider_ids();
    if enabled.is_empty() {
        println!("  Enabled providers: none");
    } else {
        println!(
            "  Enabled providers: {}",
            enabled
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!(
        "  Missing tools: {}",
        if missing_tools.is_empty() {
            "none".to_string()
        } else {
            missing_tools.join(", ")
        }
    );
    println!(
        "  Config file: {}",
        if paths.config_file.exists() {
            "present"
        } else {
            "missing"
        }
    );
    if !paths.config_file.exists() {
        println!("  Next step: run `brim config init`.");
    } else if enabled.is_empty() {
        println!(
            "  Next step: enable a provider in the config, then run `brim auth login <provider>`."
        );
    } else {
        println!("  Next step: run `brim status` or `brim sync`.");
    }
    println!();

    Ok(())
}

fn command_in_path(command: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path_var| {
        std::env::split_paths(&path_var).any(|dir| dir.join(command).exists())
    })
}
