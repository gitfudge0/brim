use brim_core::models::ProviderId;

use crate::error::StorageError;

const SERVICE_NAME: &str = "brim";

/// Wrapper around the `keyring` crate for storing provider secrets.
pub struct KeyringStore;

impl KeyringStore {
    /// Store a secret for a provider.
    pub fn set_secret(provider: ProviderId, key: &str, value: &str) -> Result<(), StorageError> {
        let entry_name = format!("{}-{}", provider.as_str(), key);
        let entry = keyring::Entry::new(SERVICE_NAME, &entry_name)
            .map_err(|e| StorageError::Keyring(e.to_string()))?;
        entry
            .set_password(value)
            .map_err(|e| StorageError::Keyring(e.to_string()))?;
        Ok(())
    }

    /// Retrieve a secret for a provider.
    pub fn get_secret(provider: ProviderId, key: &str) -> Result<Option<String>, StorageError> {
        let entry_name = format!("{}-{}", provider.as_str(), key);
        let entry = keyring::Entry::new(SERVICE_NAME, &entry_name)
            .map_err(|e| StorageError::Keyring(e.to_string()))?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(StorageError::Keyring(e.to_string())),
        }
    }

    /// Delete a secret for a provider.
    pub fn delete_secret(provider: ProviderId, key: &str) -> Result<(), StorageError> {
        let entry_name = format!("{}-{}", provider.as_str(), key);
        let entry = keyring::Entry::new(SERVICE_NAME, &entry_name)
            .map_err(|e| StorageError::Keyring(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // already gone, that's fine
            Err(e) => Err(StorageError::Keyring(e.to_string())),
        }
    }
}
