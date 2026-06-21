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
    resolve_binary_path(std::env::var_os("PREFIX"), std::env::var_os("HOME"))
        .context("cannot determine install prefix; set PREFIX or HOME")
}

// ponytail: pure so tests don't race on process-global env vars
fn resolve_binary_path(
    prefix: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let prefix = prefix
        .map(PathBuf::from)
        .or_else(|| home.map(|h| PathBuf::from(h).join(".local")))?;
    Some(prefix.join("bin").join("brim"))
}

#[cfg(test)]
mod tests {
    use super::resolve_binary_path;
    use std::path::PathBuf;

    #[test]
    fn uses_prefix_when_provided() {
        let path = resolve_binary_path(Some("/tmp/brim-prefix".into()), None).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/brim-prefix/bin/brim"));
    }

    #[test]
    fn falls_back_to_home_local_bin() {
        let path = resolve_binary_path(None, Some("/tmp/brim-home".into())).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/brim-home/.local/bin/brim"));
    }

    #[test]
    fn none_when_neither_set() {
        assert!(resolve_binary_path(None, None).is_none());
    }
}
