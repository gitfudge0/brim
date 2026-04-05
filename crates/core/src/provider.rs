use async_trait::async_trait;

use crate::error::CoreError;
use crate::models::{AuthState, ProviderId, UsageSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFlowKind {
    ExternalInstructions,
    DeviceCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthSessionState {
    Ready,
    WaitingForUser,
    Polling,
    Succeeded(AuthState),
    Failed(String),
    Cancelled,
}

impl AuthSessionState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            AuthSessionState::Succeeded(_)
                | AuthSessionState::Failed(_)
                | AuthSessionState::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSessionView {
    pub provider: ProviderId,
    pub title: String,
    pub subtitle: Option<String>,
    pub kind: AuthFlowKind,
    pub verification_uri: Option<String>,
    pub user_code: Option<String>,
    pub status_text: String,
    pub help_text: Vec<String>,
    pub can_cancel: bool,
    pub can_confirm: bool,
    pub confirm_label: Option<String>,
    pub poll_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSessionCommand {
    Start,
    Poll,
    Confirm,
    Cancel,
}

#[async_trait]
pub trait ProviderAuthSession: Send {
    fn view(&self) -> AuthSessionView;

    async fn advance(&mut self, command: AuthSessionCommand)
        -> Result<AuthSessionState, CoreError>;
}

/// The core trait that every provider (codex, claude, copilot) must implement.
///
/// Each provider owns:
/// - Its identity and display metadata
/// - Its authentication flow(s)
/// - Its fetch strategies (ordered by preference)
/// - Parsing raw API responses into normalized `UsageSnapshot`
#[async_trait]
pub trait Provider: Send + Sync {
    /// Which provider this is.
    fn id(&self) -> ProviderId;

    /// Human-readable name for display.
    fn display_name(&self) -> &str;

    /// Check the current auth state without making network requests.
    async fn auth_state(&self) -> AuthState;

    /// Attempt to authenticate or refresh credentials.
    /// Returns the new auth state.
    async fn authenticate(&self) -> Result<AuthState, CoreError>;

    /// Begin a UI-managed authentication session.
    fn begin_auth_session(&self) -> Result<Box<dyn ProviderAuthSession>, CoreError>;

    /// Fetch a fresh usage snapshot.
    ///
    /// Implementations should try strategies in order of preference:
    /// 1. Official API (if available)
    /// 2. CLI/local file probing
    /// 3. Experimental/internal API
    ///
    /// The returned snapshot must have all values correctly labeled with
    /// their confidence level.
    async fn fetch_usage(&self) -> Result<UsageSnapshot, CoreError>;

    /// List the names of fetch strategies this provider supports,
    /// in order of preference.
    fn strategies(&self) -> Vec<&str>;
}

/// A registry of all available providers.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.push(provider);
    }

    pub fn get(&self, id: ProviderId) -> Option<&dyn Provider> {
        self.providers
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    pub fn all(&self) -> &[Box<dyn Provider>] {
        &self.providers
    }

    pub fn ids(&self) -> Vec<ProviderId> {
        self.providers.iter().map(|p| p.id()).collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
