use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

use brim_core::models::{ProviderId, UsageSnapshot};

use crate::error::StorageError;

/// SQLite-backed storage for usage snapshots and metadata.
///
/// Wraps `Connection` in a `Mutex` so `Database` is `Send + Sync`,
/// allowing safe sharing via `Arc<Database>` across async tasks.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) the database at the given path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("database mutex poisoned")
    }

    fn migrate(&self) -> Result<(), StorageError> {
        self.conn().execute_batch(
            "
            CREATE TABLE IF NOT EXISTS usage_snapshots (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                provider    TEXT NOT NULL,
                fetched_at  TEXT NOT NULL,
                strategy    TEXT NOT NULL,
                data_json   TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE INDEX IF NOT EXISTS idx_snapshots_provider_time
                ON usage_snapshots(provider, fetched_at DESC);

            CREATE TABLE IF NOT EXISTS auth_state (
                provider    TEXT PRIMARY KEY,
                state       TEXT NOT NULL,
                updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE IF NOT EXISTS kv_meta (
                key         TEXT PRIMARY KEY,
                value       TEXT NOT NULL,
                updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            ",
        )?;
        Ok(())
    }

    /// Store a usage snapshot.
    pub fn insert_snapshot(&self, snapshot: &UsageSnapshot) -> Result<i64, StorageError> {
        let json = serde_json::to_string(snapshot)?;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO usage_snapshots (provider, fetched_at, strategy, data_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                snapshot.provider.as_str(),
                snapshot.fetched_at.to_rfc3339(),
                snapshot.source_strategy,
                json,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get the latest snapshot for a provider.
    pub fn latest_snapshot(
        &self,
        provider: ProviderId,
    ) -> Result<Option<UsageSnapshot>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT data_json FROM usage_snapshots
             WHERE provider = ?1
             ORDER BY fetched_at DESC
             LIMIT 1",
        )?;
        let result = stmt.query_row(params![provider.as_str()], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        });

        match result {
            Ok(json) => {
                let snapshot: UsageSnapshot = serde_json::from_str(&json)?;
                Ok(Some(snapshot))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get snapshots for a provider within a time range.
    pub fn snapshots_in_range(
        &self,
        provider: ProviderId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<UsageSnapshot>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT data_json FROM usage_snapshots
             WHERE provider = ?1 AND fetched_at >= ?2 AND fetched_at <= ?3
             ORDER BY fetched_at ASC",
        )?;

        let rows = stmt.query_map(
            params![provider.as_str(), from.to_rfc3339(), to.to_rfc3339()],
            |row| {
                let json: String = row.get(0)?;
                Ok(json)
            },
        )?;

        let mut snapshots = Vec::new();
        for row in rows {
            let json = row?;
            let snapshot: UsageSnapshot = serde_json::from_str(&json)?;
            snapshots.push(snapshot);
        }
        Ok(snapshots)
    }

    /// Delete snapshots older than a given age.
    pub fn prune_snapshots(&self, older_than: DateTime<Utc>) -> Result<usize, StorageError> {
        let deleted = self.conn().execute(
            "DELETE FROM usage_snapshots WHERE fetched_at < ?1",
            params![older_than.to_rfc3339()],
        )?;
        Ok(deleted)
    }

    /// Store a key-value metadata entry.
    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.conn().execute(
            "INSERT INTO kv_meta (key, value, updated_at)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get a key-value metadata entry.
    pub fn get_meta(&self, key: &str) -> Result<Option<String>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT value FROM kv_meta WHERE key = ?1")?;
        let result = stmt.query_row(params![key], |row| row.get(0));
        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brim_core::confidence::Labeled;
    use brim_core::models::QuotaBucket;
    use brim_core::time_window::TimeWindow;

    fn make_test_snapshot() -> UsageSnapshot {
        UsageSnapshot {
            provider: ProviderId::Codex,
            fetched_at: Utc::now(),
            plan: None,
            buckets: vec![QuotaBucket {
                metric: "session".into(),
                label: "Session Usage".into(),
                window: TimeWindow::session("5-hour session", 5 * 3600),
                used: Some(Labeled::experimental(10.0)),
                limit: Some(Labeled::experimental(100.0)),
                percent_remaining: None,
            }],
            source_strategy: "test".into(),
            notes: vec![],
        }
    }

    #[test]
    fn test_insert_and_retrieve_snapshot() {
        let db = Database::open_memory().unwrap();
        let snapshot = make_test_snapshot();
        let id = db.insert_snapshot(&snapshot).unwrap();
        assert!(id > 0);

        let retrieved = db.latest_snapshot(ProviderId::Codex).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.provider, ProviderId::Codex);
        assert_eq!(retrieved.buckets.len(), 1);
    }

    #[test]
    fn test_no_snapshot_returns_none() {
        let db = Database::open_memory().unwrap();
        let result = db.latest_snapshot(ProviderId::Claude).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_meta_kv() {
        let db = Database::open_memory().unwrap();
        db.set_meta("last_sync", "2025-01-01T00:00:00Z").unwrap();
        let val = db.get_meta("last_sync").unwrap();
        assert_eq!(val, Some("2025-01-01T00:00:00Z".to_string()));

        let missing = db.get_meta("nonexistent").unwrap();
        assert!(missing.is_none());
    }
}
