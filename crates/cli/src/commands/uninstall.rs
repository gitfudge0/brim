use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn run() -> Result<()> {
    // Remove the background service first so we don't leave an orphaned unit
    // pointing at a deleted binary.
    if let Err(e) = crate::service::uninstall() {
        eprintln!("warning: could not remove auto-sync service: {e}");
    }

    let path = installed_binary_path()?;

    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("remove installed binary at {}", path.display()))?;
        println!("Removed {}", path.display());
    } else {
        println!("No installed brim binary found at {}", path.display());
    }

    println!("Config, state, and credentials were left untouched.");
    Ok(())
}

fn installed_binary_path() -> Result<PathBuf> {
    let prefix = std::env::var_os("PREFIX")
        .map(PathBuf::from)
        .or_else(default_prefix)
        .context("cannot determine install prefix; set PREFIX or HOME")?;

    Ok(prefix.join("bin").join("brim"))
}

fn default_prefix() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local"))
}

#[cfg(test)]
mod tests {
    use super::installed_binary_path;
    use std::path::PathBuf;

    #[test]
    fn uses_prefix_when_provided() {
        unsafe {
            std::env::set_var("PREFIX", "/tmp/brim-prefix");
            std::env::remove_var("HOME");
        }

        let path = installed_binary_path().unwrap();
        assert_eq!(path, PathBuf::from("/tmp/brim-prefix/bin/brim"));

        unsafe {
            std::env::remove_var("PREFIX");
        }
    }

    #[test]
    fn falls_back_to_home_local_bin() {
        unsafe {
            std::env::remove_var("PREFIX");
            std::env::set_var("HOME", "/tmp/brim-home");
        }

        let path = installed_binary_path().unwrap();
        assert_eq!(path, PathBuf::from("/tmp/brim-home/.local/bin/brim"));

        unsafe {
            std::env::remove_var("HOME");
        }
    }
}
