use std::sync::Arc;

use chrono::Utc;
use tracing::{debug, error, info, warn};

use brim_core::error::CoreError;
use brim_core::models::{AuthState, ProviderId, ProviderStatus, UsageSnapshot};
use brim_core::provider::ProviderRegistry;
use brim_storage::config::AppConfig;
use brim_storage::db::Database;

/// The sync engine orchestrates fetching usage from all providers,
/// storing results in the database, and providing cached data.
pub struct SyncEngine {
    registry: ProviderRegistry,
    db: Arc<Database>,
    config: Arc<AppConfig>,
}

/// Result of a single provider sync.
#[derive(Debug)]
pub struct SyncResult {
    pub provider: ProviderId,
    pub snapshot: Option<UsageSnapshot>,
    pub failure: Option<SyncFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFailure {
    pub kind: SyncFailureKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncFailureKind {
    Auth,
    RateLimited,
    Fetch,
}

impl SyncFailure {
    fn from_error(error: &CoreError) -> Self {
        let kind = match error {
            CoreError::AuthFailed { .. } => SyncFailureKind::Auth,
            CoreError::RateLimited { .. } => SyncFailureKind::RateLimited,
            _ => SyncFailureKind::Fetch,
        };

        Self {
            kind,
            message: error.to_string(),
        }
    }
}

impl SyncEngine {
    pub fn new(registry: ProviderRegistry, db: Arc<Database>, config: Arc<AppConfig>) -> Self {
        Self {
            registry,
            db,
            config,
        }
    }

    /// Sync a single provider: fetch usage and store in DB.
    pub async fn sync_provider(&self, id: ProviderId) -> SyncResult {
        let provider = match self.registry.get(id) {
            Some(p) => p,
            None => {
                return SyncResult {
                    provider: id,
                    snapshot: None,
                    failure: Some(SyncFailure {
                        kind: SyncFailureKind::Fetch,
                        message: format!("provider {} not registered", id.as_str()),
                    }),
                }
            }
        };

        info!("Syncing {}...", provider.display_name());

        match provider.fetch_usage().await {
            Ok(snapshot) => {
                // Store in DB
                match self.db.insert_snapshot(&snapshot) {
                    Ok(row_id) => {
                        debug!(
                            "Stored snapshot for {} (row {}): {} buckets",
                            id.as_str(),
                            row_id,
                            snapshot.buckets.len()
                        );
                    }
                    Err(e) => {
                        error!("Failed to store snapshot for {}: {}", id.as_str(), e);
                    }
                }

                SyncResult {
                    provider: id,
                    snapshot: Some(snapshot),
                    failure: None,
                }
            }
            Err(e) => {
                warn!("Sync failed for {}: {}", id.as_str(), e);
                SyncResult {
                    provider: id,
                    snapshot: None,
                    failure: Some(SyncFailure::from_error(&e)),
                }
            }
        }
    }

    /// Sync all enabled providers, then prune old data.
    pub async fn sync_all(&self) -> Vec<SyncResult> {
        let ids = self.registry.ids();
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            results.push(self.sync_provider(id).await);
        }
        // Auto-prune old data
        self.prune_old_data(self.config.general.prune_after_days as i64);
        results
    }

    /// Get the latest cached snapshot for a provider (from DB).
    /// If the snapshot is older than the configured stale threshold,
    /// all values are marked with Stale confidence.
    pub fn cached_snapshot(&self, id: ProviderId) -> Option<UsageSnapshot> {
        match self.db.latest_snapshot(id) {
            Ok(Some(snap)) => {
                let stale_secs = self.config.general.stale_threshold_secs;
                if snap.is_older_than(stale_secs) {
                    debug!(
                        "Snapshot for {} is stale (older than {}s)",
                        id.as_str(),
                        stale_secs
                    );
                    Some(snap.mark_stale())
                } else {
                    Some(snap)
                }
            }
            Ok(None) => None,
            Err(e) => {
                warn!("Failed to read cached snapshot for {}: {}", id.as_str(), e);
                None
            }
        }
    }

    /// Get status for all registered providers.
    pub async fn all_statuses(&self) -> Vec<ProviderStatus> {
        let mut statuses = Vec::new();
        for provider in self.registry.all() {
            let auth = provider.auth_state().await;
            let last_snapshot = self.cached_snapshot(provider.id());
            let enabled = self.config.provider(provider.id()).enabled;
            statuses.push(ProviderStatus {
                provider: provider.id(),
                auth_state: auth,
                last_snapshot,
                enabled,
            });
        }
        statuses
    }

    /// Get a fresh status for a single provider by combining a live sync attempt
    /// with the cached snapshot and local auth state.
    pub async fn fresh_status(&self, id: ProviderId) -> ProviderStatus {
        let enabled = self.config.provider(id).enabled;
        let result = self.sync_provider(id).await;

        if let Some(snapshot) = result.snapshot {
            return ProviderStatus {
                provider: id,
                auth_state: AuthState::Authenticated,
                last_snapshot: Some(snapshot),
                enabled,
            };
        }

        let cached_snapshot = self.cached_snapshot(id);

        let auth_state = match result.failure {
            Some(SyncFailure {
                kind: SyncFailureKind::Auth,
                message,
            }) => AuthState::Failed(message),
            Some(_) | None => match self.registry.get(id) {
                Some(provider) => provider.auth_state().await,
                None => AuthState::NotConfigured,
            },
        };

        ProviderStatus {
            provider: id,
            auth_state,
            last_snapshot: cached_snapshot,
            enabled,
        }
    }

    /// Get fresh statuses for all registered providers.
    pub async fn fresh_statuses(&self) -> Vec<ProviderStatus> {
        let ids = self.registry.ids();
        let mut statuses = Vec::with_capacity(ids.len());
        for id in ids {
            statuses.push(self.fresh_status(id).await);
        }
        self.prune_old_data(self.config.general.prune_after_days as i64);
        statuses
    }

    /// Prune old snapshots from the database.
    pub fn prune_old_data(&self, max_age_days: i64) -> usize {
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days);
        match self.db.prune_snapshots(cutoff) {
            Ok(n) => {
                if n > 0 {
                    info!("Pruned {} old snapshots", n);
                }
                n
            }
            Err(e) => {
                error!("Failed to prune snapshots: {}", e);
                0
            }
        }
    }

    /// Access the registry.
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Access the database.
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Access the application config.
    pub fn config(&self) -> &AppConfig {
        self.config.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use brim_core::confidence::Labeled;
    use brim_core::provider::{
        AuthFlowKind, AuthSessionCommand, AuthSessionState, AuthSessionView, Provider,
        ProviderAuthSession,
    };
    use brim_core::time_window::TimeWindow;

    enum TestFetchResult {
        Success(UsageSnapshot),
        AuthFailure(&'static str),
        FetchFailure(&'static str),
    }

    struct TestProvider {
        id: ProviderId,
        auth_state: AuthState,
        fetch_result: TestFetchResult,
    }

    #[async_trait]
    impl Provider for TestProvider {
        fn id(&self) -> ProviderId {
            self.id
        }

        fn display_name(&self) -> &str {
            "Test Provider"
        }

        async fn auth_state(&self) -> AuthState {
            self.auth_state.clone()
        }

        async fn authenticate(&self) -> Result<AuthState, CoreError> {
            Ok(self.auth_state.clone())
        }

        fn begin_auth_session(&self) -> Result<Box<dyn ProviderAuthSession>, CoreError> {
            Ok(Box::new(TestAuthSession))
        }

        async fn fetch_usage(&self) -> Result<UsageSnapshot, CoreError> {
            match &self.fetch_result {
                TestFetchResult::Success(snapshot) => Ok(snapshot.clone()),
                TestFetchResult::AuthFailure(reason) => Err(CoreError::AuthFailed {
                    provider: self.id.as_str().into(),
                    reason: (*reason).into(),
                }),
                TestFetchResult::FetchFailure(reason) => Err(CoreError::FetchFailed {
                    provider: self.id.as_str().into(),
                    reason: (*reason).into(),
                }),
            }
        }

        fn strategies(&self) -> Vec<&str> {
            vec!["test"]
        }
    }

    struct TestAuthSession;

    #[async_trait]
    impl ProviderAuthSession for TestAuthSession {
        fn view(&self) -> AuthSessionView {
            AuthSessionView {
                provider: ProviderId::Copilot,
                title: "Test".into(),
                subtitle: None,
                kind: AuthFlowKind::ExternalInstructions,
                verification_uri: None,
                user_code: None,
                status_text: "Test".into(),
                help_text: vec![],
                can_cancel: true,
                can_confirm: true,
                confirm_label: Some("Close".into()),
                poll_interval_secs: None,
            }
        }

        async fn advance(
            &mut self,
            _command: AuthSessionCommand,
        ) -> Result<AuthSessionState, CoreError> {
            Ok(AuthSessionState::Cancelled)
        }
    }

    fn test_snapshot(provider: ProviderId, strategy: &str) -> UsageSnapshot {
        UsageSnapshot {
            provider,
            fetched_at: Utc::now(),
            plan: None,
            buckets: vec![brim_core::models::QuotaBucket {
                metric: "requests".into(),
                label: "Requests".into(),
                window: TimeWindow::session("Test", 60),
                used: Some(Labeled::experimental(1.0)),
                limit: Some(Labeled::experimental(10.0)),
                percent_remaining: None,
            }],
            source_strategy: strategy.into(),
            notes: vec![],
        }
    }

    fn engine_with_provider(provider: TestProvider, enabled: bool) -> SyncEngine {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(provider));

        let mut config = AppConfig::default();
        config.set_provider_enabled(ProviderId::Copilot, enabled);

        SyncEngine::new(
            registry,
            Arc::new(Database::open_memory().expect("in-memory db")),
            Arc::new(config),
        )
    }

    #[tokio::test]
    async fn fresh_status_returns_authenticated_on_success() {
        let snapshot = test_snapshot(ProviderId::Copilot, "fresh");
        let engine = engine_with_provider(
            TestProvider {
                id: ProviderId::Copilot,
                auth_state: AuthState::Configured,
                fetch_result: TestFetchResult::Success(snapshot.clone()),
            },
            true,
        );

        let status = engine.fresh_status(ProviderId::Copilot).await;

        assert_eq!(status.auth_state, AuthState::Authenticated);
        assert_eq!(
            status
                .last_snapshot
                .as_ref()
                .map(|s| s.source_strategy.as_str()),
            Some("fresh")
        );
    }

    #[tokio::test]
    async fn fresh_status_preserves_cached_snapshot_on_auth_failure() {
        let cached = test_snapshot(ProviderId::Copilot, "cached");
        let engine = engine_with_provider(
            TestProvider {
                id: ProviderId::Copilot,
                auth_state: AuthState::Configured,
                fetch_result: TestFetchResult::AuthFailure("token rejected"),
            },
            true,
        );
        engine.db.insert_snapshot(&cached).expect("cached snapshot");

        let status = engine.fresh_status(ProviderId::Copilot).await;

        assert_eq!(
            status.auth_state,
            AuthState::Failed("authentication failed for copilot: token rejected".into())
        );
        assert_eq!(
            status
                .last_snapshot
                .as_ref()
                .map(|s| s.source_strategy.as_str()),
            Some("cached")
        );
    }

    #[tokio::test]
    async fn fresh_status_keeps_local_auth_state_on_non_auth_failure() {
        let cached = test_snapshot(ProviderId::Copilot, "cached");
        let engine = engine_with_provider(
            TestProvider {
                id: ProviderId::Copilot,
                auth_state: AuthState::Configured,
                fetch_result: TestFetchResult::FetchFailure("network down"),
            },
            true,
        );
        engine.db.insert_snapshot(&cached).expect("cached snapshot");

        let status = engine.fresh_status(ProviderId::Copilot).await;

        assert_eq!(status.auth_state, AuthState::Configured);
        assert_eq!(
            status
                .last_snapshot
                .as_ref()
                .map(|s| s.source_strategy.as_str()),
            Some("cached")
        );
    }

    #[tokio::test]
    async fn fresh_status_handles_auth_failure_without_cached_snapshot() {
        let engine = engine_with_provider(
            TestProvider {
                id: ProviderId::Copilot,
                auth_state: AuthState::Configured,
                fetch_result: TestFetchResult::AuthFailure("device token expired"),
            },
            true,
        );

        let status = engine.fresh_status(ProviderId::Copilot).await;

        assert_eq!(
            status.auth_state,
            AuthState::Failed("authentication failed for copilot: device token expired".into())
        );
        assert!(status.last_snapshot.is_none());
    }
}
