use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use brim_core::models::AuthState;
use brim_core::models::{ProviderId, ProviderStatus};
use brim_core::provider::{AuthSessionState, AuthSessionView};

/// What action the main loop should take after processing a key.
pub enum AppAction {
    None,
    Quit,
    Refresh,
    OpenAddProvider,
    OpenAuthPopup(ProviderId),
    RemoveProvider(ProviderId),
    ConfirmSetup(ProviderId),
    Back,
    AuthModalConfirm,
    AuthModalCancel,
    AuthModalClose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    ProviderPicker,
    ProviderSetup(ProviderId),
}

pub struct AuthModalState {
    pub provider: ProviderId,
    pub view: AuthSessionView,
    pub status: AuthSessionState,
    pub pending_refresh_after_close: bool,
}

/// Application state for the TUI.
pub struct App {
    /// Status data for each enabled provider.
    pub statuses: Vec<ProviderStatus>,
    /// Which provider panel is currently focused (0-indexed).
    pub selected: usize,
    /// Whether a sync is currently in progress.
    pub syncing: bool,
    /// Status message shown in the footer.
    pub message: String,
    /// Current UI screen.
    pub screen: Screen,
    /// Selection index within the provider picker.
    pub picker_selected: usize,
    /// Providers that may still be added.
    pub available_providers: Vec<ProviderId>,
    /// Active modal popup, if any.
    pub modal: Option<AuthModalState>,
}

impl App {
    pub fn new(statuses: Vec<ProviderStatus>, available_providers: Vec<ProviderId>) -> Self {
        let screen = if statuses.is_empty() {
            Screen::ProviderPicker
        } else {
            Screen::Dashboard
        };

        let message = match screen {
            Screen::Dashboard => "Press 'a' to add a service, 'r' to refresh, 'q' to quit".into(),
            Screen::ProviderPicker => "Select a service to track. Press Enter to continue.".into(),
            Screen::ProviderSetup(_) => String::new(),
        };

        Self {
            selected: 0,
            syncing: false,
            message,
            statuses,
            screen,
            picker_selected: 0,
            available_providers,
            modal: None,
        }
    }

    pub fn update_statuses(
        &mut self,
        statuses: Vec<ProviderStatus>,
        available_providers: Vec<ProviderId>,
    ) {
        self.statuses = statuses;
        self.available_providers = available_providers;

        if self.statuses.is_empty() {
            self.screen = Screen::ProviderPicker;
            self.selected = 0;
            self.message = "Select a service to track. Press Enter to continue.".into();
        } else if matches!(self.screen, Screen::ProviderPicker) {
            self.screen = Screen::Dashboard;
            self.message = format!("Last synced: {}", chrono::Utc::now().format("%H:%M:%S UTC"));
        } else if self.modal.is_none() {
            self.message = format!("Last synced: {}", chrono::Utc::now().format("%H:%M:%S UTC"));
        }

        if !self.statuses.is_empty() {
            self.selected = self.selected.min(self.statuses.len().saturating_sub(1));
        }
        if !self.available_providers.is_empty() {
            self.picker_selected = self
                .picker_selected
                .min(self.available_providers.len().saturating_sub(1));
        } else {
            self.picker_selected = 0;
        }
    }

    pub fn set_syncing(&mut self, syncing: bool) {
        self.syncing = syncing;
        if syncing && self.modal.is_none() {
            self.message = "Syncing...".into();
        }
    }

    pub fn open_picker(&mut self) {
        self.screen = Screen::ProviderPicker;
        self.message = "Select a service to add. Press Enter to continue.".into();
    }

    pub fn open_setup(&mut self, provider: ProviderId) {
        self.screen = Screen::ProviderSetup(provider);
        self.message = format!("Set up {}", provider.display_name());
    }

    pub fn open_auth_modal(
        &mut self,
        provider: ProviderId,
        view: AuthSessionView,
        status: AuthSessionState,
    ) {
        self.screen = Screen::Dashboard;
        self.modal = Some(AuthModalState {
            provider,
            view,
            pending_refresh_after_close: matches!(&status, AuthSessionState::Succeeded(auth) if auth.is_usable()),
            status,
        });
        self.message = format!("Authenticating {}", provider.display_name());
    }

    pub fn update_auth_modal(&mut self, view: AuthSessionView, status: AuthSessionState) {
        if let Some(modal) = &mut self.modal {
            modal.view = view;
            modal.pending_refresh_after_close =
                matches!(&status, AuthSessionState::Succeeded(auth) if auth.is_usable());
            modal.status = status;
        }
    }

    pub fn close_auth_modal(&mut self) -> Option<AuthModalState> {
        self.modal.take()
    }

    pub fn back_to_dashboard_or_picker(&mut self) {
        if self.statuses.is_empty() {
            self.screen = Screen::ProviderPicker;
            if self.message.is_empty() {
                self.message = "Select a service to track. Press Enter to continue.".into();
            }
        } else {
            self.screen = Screen::Dashboard;
            if self.message.is_empty() {
                self.message = "Press 'a' to add a service, 'r' to refresh, 'q' to quit".into();
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if self.modal.is_some() {
            return self.handle_modal_key(key);
        }

        match self.screen {
            Screen::Dashboard => self.handle_dashboard_key(key),
            Screen::ProviderPicker => self.handle_picker_key(key),
            Screen::ProviderSetup(provider) => self.handle_setup_key(key, provider),
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('q') => AppAction::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => AppAction::Quit,
            KeyCode::Esc => {
                if self
                    .modal
                    .as_ref()
                    .map(|modal| modal.status.is_terminal())
                    .unwrap_or(false)
                {
                    AppAction::AuthModalClose
                } else {
                    AppAction::AuthModalCancel
                }
            }
            KeyCode::Enter => {
                if self
                    .modal
                    .as_ref()
                    .map(|modal| modal.status.is_terminal())
                    .unwrap_or(false)
                {
                    AppAction::AuthModalClose
                } else {
                    AppAction::AuthModalConfirm
                }
            }
            _ => AppAction::None,
        }
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => AppAction::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => AppAction::Quit,
            KeyCode::Char('r') | KeyCode::F(5) => AppAction::Refresh,
            KeyCode::Char('a') => {
                if self.available_providers.is_empty() {
                    self.message = "All supported services are already added.".into();
                    AppAction::None
                } else {
                    AppAction::OpenAddProvider
                }
            }
            KeyCode::Char('x') => self
                .statuses
                .get(self.selected)
                .map(|status| AppAction::RemoveProvider(status.provider))
                .unwrap_or(AppAction::None),
            KeyCode::Char('e') => self
                .statuses
                .get(self.selected)
                .filter(|status| {
                    matches!(status.auth_state, AuthState::Failed(_) | AuthState::Expired)
                })
                .map(|status| AppAction::OpenAuthPopup(status.provider))
                .unwrap_or_else(|| {
                    self.message = "Selected service does not need re-authentication.".into();
                    AppAction::None
                }),
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                if !self.statuses.is_empty() {
                    self.selected = (self.selected + 1) % self.statuses.len();
                }
                AppAction::None
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                if !self.statuses.is_empty() {
                    self.selected = self
                        .selected
                        .checked_sub(1)
                        .unwrap_or(self.statuses.len() - 1);
                }
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('q') => AppAction::Quit,
            KeyCode::Esc => {
                if self.statuses.is_empty() {
                    AppAction::Quit
                } else {
                    AppAction::Back
                }
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => AppAction::Quit,
            KeyCode::Up | KeyCode::Left => {
                if !self.available_providers.is_empty() {
                    self.picker_selected = self
                        .picker_selected
                        .checked_sub(1)
                        .unwrap_or(self.available_providers.len() - 1);
                }
                AppAction::None
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Tab => {
                if !self.available_providers.is_empty() {
                    self.picker_selected =
                        (self.picker_selected + 1) % self.available_providers.len();
                }
                AppAction::None
            }
            KeyCode::Enter => self
                .available_providers
                .get(self.picker_selected)
                .copied()
                .map(|provider| {
                    self.open_setup(provider);
                    AppAction::None
                })
                .unwrap_or(AppAction::None),
            _ => AppAction::None,
        }
    }

    fn handle_setup_key(&mut self, key: KeyEvent, provider: ProviderId) -> AppAction {
        match key.code {
            KeyCode::Char('q') => AppAction::Quit,
            KeyCode::Esc => AppAction::Back,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => AppAction::Quit,
            KeyCode::Enter | KeyCode::Char('c') => AppAction::ConfirmSetup(provider),
            _ => AppAction::None,
        }
    }

    #[allow(dead_code)]
    pub fn provider_count(&self) -> usize {
        self.statuses.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brim_core::provider::AuthFlowKind;

    fn status(provider: ProviderId, auth_state: AuthState) -> ProviderStatus {
        ProviderStatus {
            provider,
            auth_state,
            last_snapshot: None,
            enabled: true,
        }
    }

    fn modal_view() -> AuthSessionView {
        AuthSessionView {
            provider: ProviderId::Copilot,
            title: "GitHub Device Authorization".into(),
            subtitle: None,
            kind: AuthFlowKind::DeviceCode,
            verification_uri: Some("https://github.com/login/device".into()),
            user_code: Some("ABCD-1234".into()),
            status_text: "Waiting".into(),
            help_text: vec![],
            can_cancel: true,
            can_confirm: true,
            confirm_label: Some("Continue".into()),
            poll_interval_secs: Some(5),
        }
    }

    #[test]
    fn dashboard_opens_auth_popup_for_failed_provider() {
        let mut app = App::new(
            vec![status(
                ProviderId::Copilot,
                AuthState::Failed("token rejected".into()),
            )],
            vec![],
        );

        let action = app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

        assert!(matches!(
            action,
            AppAction::OpenAuthPopup(ProviderId::Copilot)
        ));
    }

    #[test]
    fn dashboard_does_not_emit_reauthenticate_for_healthy_provider() {
        let mut app = App::new(
            vec![status(ProviderId::Copilot, AuthState::Configured)],
            vec![],
        );

        let action = app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

        assert!(matches!(action, AppAction::None));
        assert_eq!(
            app.message,
            "Selected service does not need re-authentication."
        );
    }

    #[test]
    fn modal_consumes_enter_and_suppresses_dashboard_actions() {
        let mut app = App::new(
            vec![status(ProviderId::Copilot, AuthState::Configured)],
            vec![],
        );
        app.open_auth_modal(
            ProviderId::Copilot,
            modal_view(),
            AuthSessionState::WaitingForUser,
        );

        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(action, AppAction::AuthModalConfirm));
    }
}
