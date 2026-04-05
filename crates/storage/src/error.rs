use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("config error: {0}")]
    Config(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("path error: {0}")]
    Path(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Database(e.to_string())
    }
}

impl From<toml::de::Error> for StorageError {
    fn from(e: toml::de::Error) -> Self {
        StorageError::Config(e.to_string())
    }
}

impl From<toml::ser::Error> for StorageError {
    fn from(e: toml::ser::Error) -> Self {
        StorageError::Serialization(e.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Serialization(e.to_string())
    }
}
