use anyhow::Result;
use brim_storage::config::AppConfig;
use brim_storage::paths::AppPaths;

pub fn show() -> Result<()> {
    let paths = AppPaths::resolve().map_err(|e| anyhow::anyhow!("{}", e))?;
    let config = AppConfig::load(&paths.config_file).map_err(|e| anyhow::anyhow!("{}", e))?;
    let toml_str =
        toml::to_string_pretty(&config).map_err(|e| anyhow::anyhow!("serialize: {}", e))?;
    println!("# Config file: {}", paths.config_file.display());
    println!("{}", toml_str);
    Ok(())
}

pub fn init() -> Result<()> {
    let paths = AppPaths::resolve().map_err(|e| anyhow::anyhow!("{}", e))?;
    paths.ensure_dirs().map_err(|e| anyhow::anyhow!("{}", e))?;

    if paths.config_file.exists() {
        println!("Config already exists at {}", paths.config_file.display());
        return Ok(());
    }

    let config = AppConfig::default();
    config
        .save(&paths.config_file)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Created default config at {}", paths.config_file.display());
    Ok(())
}

pub fn edit() -> Result<()> {
    let paths = AppPaths::resolve().map_err(|e| anyhow::anyhow!("{}", e))?;
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let status = std::process::Command::new(&editor)
        .arg(&paths.config_file)
        .status()?;
    if !status.success() {
        anyhow::bail!("editor exited with {}", status);
    }
    Ok(())
}
