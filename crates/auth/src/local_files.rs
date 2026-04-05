use std::path::{Path, PathBuf};

use crate::error::AuthError;

/// Discover local credential files from known provider CLI locations.
///
/// Try to find the Codex CLI auth file.
/// Locations: ~/.codex/auth.json
pub fn find_codex_auth_file() -> Option<PathBuf> {
    let home = dirs_next::home_dir()?;
    let path = home.join(".codex").join("auth.json");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Try to find the Claude CLI credentials file.
/// Locations: ~/.claude/.credentials.json
pub fn find_claude_credentials_file() -> Option<PathBuf> {
    let home = dirs_next::home_dir()?;
    let path = home.join(".claude").join(".credentials.json");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Read a JSON file and parse it.
pub fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, AuthError> {
    let contents = std::fs::read_to_string(path)?;
    let parsed: T = serde_json::from_str(&contents)?;
    Ok(parsed)
}
