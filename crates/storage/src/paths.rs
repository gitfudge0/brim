use directories::ProjectDirs;
use std::path::PathBuf;

use crate::error::StorageError;

const QUALIFIER: &str = "";
const ORG: &str = "";
const APP: &str = "brim";

/// Resolved XDG-compliant paths for the application.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// ~/.config/brim/
    pub config_dir: PathBuf,
    /// ~/.config/brim/config.toml
    pub config_file: PathBuf,
    /// ~/.local/state/brim/
    pub state_dir: PathBuf,
    /// ~/.local/state/brim/app.db
    pub db_file: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self, StorageError> {
        let dirs = ProjectDirs::from(QUALIFIER, ORG, APP)
            .ok_or_else(|| StorageError::Path("cannot determine home directory".into()))?;

        let config_dir = dirs.config_dir().to_path_buf();
        let config_file = config_dir.join("config.toml");

        // ProjectDirs doesn't give state_dir on all platforms;
        // we use data_local_dir and override to ~/.local/state/ on Linux.
        let state_dir = if cfg!(target_os = "linux") {
            dirs.data_local_dir()
                .parent()
                .unwrap_or(dirs.data_local_dir())
                .join("state")
                .join(APP)
        } else {
            dirs.data_local_dir().to_path_buf()
        };
        let db_file = state_dir.join("app.db");

        Ok(Self {
            config_dir,
            config_file,
            state_dir,
            db_file,
        })
    }

    /// Ensure all directories exist.
    pub fn ensure_dirs(&self) -> Result<(), StorageError> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.state_dir)?;
        Ok(())
    }
}
