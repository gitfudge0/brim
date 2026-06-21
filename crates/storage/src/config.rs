use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use brim_core::models::ProviderId;

use crate::error::StorageError;

/// Top-level application configuration (stored as TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub providers: HashMap<String, ProviderConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut providers = HashMap::new();
        for id in ProviderId::all() {
            providers.insert(
                id.as_str().to_string(),
                ProviderConfig {
                    enabled: false,
                    poll_interval_secs: 300,
                    extra: HashMap::new(),
                },
            );
        }
        Self {
            general: GeneralConfig::default(),
            providers,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Default poll interval in seconds.
    pub poll_interval_secs: u64,
    /// Data older than this many seconds is considered stale.
    pub stale_threshold_secs: u64,
    /// Snapshots older than this many days are pruned from the database.
    pub prune_after_days: u64,
    /// Log level filter (e.g. "info", "debug", "brim_providers=debug").
    pub log_level: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 300,
            stale_threshold_secs: 600, // 10 minutes
            prune_after_days: 30,
            log_level: "info".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub enabled: bool,
    pub poll_interval_secs: u64,
    /// Provider-specific key-value settings.
    pub extra: HashMap<String, String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_secs: 300,
            extra: HashMap::new(),
        }
    }
}

impl AppConfig {
    /// Load config from a TOML file, or return defaults if it doesn't exist.
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Save config to a TOML file.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let contents = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Get the config for a specific provider.
    pub fn provider(&self, id: ProviderId) -> ProviderConfig {
        self.providers.get(id.as_str()).cloned().unwrap_or_default()
    }

    pub fn set_provider_enabled(&mut self, id: ProviderId, enabled: bool) {
        self.providers
            .entry(id.as_str().to_string())
            .or_default()
            .enabled = enabled;
    }

    /// Set the poll interval. `id = None` sets the general default; otherwise
    /// sets a per-provider override.
    pub fn set_poll_interval(&mut self, id: Option<ProviderId>, secs: u64) {
        match id {
            None => self.general.poll_interval_secs = secs,
            Some(id) => {
                self.providers
                    .entry(id.as_str().to_string())
                    .or_default()
                    .poll_interval_secs = secs
            }
        }
    }

    pub fn enabled_provider_ids(&self) -> Vec<ProviderId> {
        ProviderId::all()
            .iter()
            .copied()
            .filter(|id| self.provider(*id).enabled)
            .collect()
    }

    pub fn has_enabled_providers(&self) -> bool {
        !self.enabled_provider_ids().is_empty()
    }

    /// Get the effective poll interval for a provider.
    pub fn poll_interval(&self, id: ProviderId) -> u64 {
        let provider = self.provider(id);
        if provider.poll_interval_secs > 0 {
            provider.poll_interval_secs
        } else {
            self.general.poll_interval_secs
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_poll_interval_general_and_override() {
        let mut config = AppConfig::default();

        config.set_poll_interval(None, 600);
        assert_eq!(config.general.poll_interval_secs, 600);

        // A per-provider override wins over the general default.
        config.set_poll_interval(Some(ProviderId::Claude), 900);
        assert_eq!(config.poll_interval(ProviderId::Claude), 900);

        // A provider explicitly set to 0 falls back to the general default.
        config.set_poll_interval(Some(ProviderId::Codex), 0);
        assert_eq!(config.poll_interval(ProviderId::Codex), 600);
    }
}
