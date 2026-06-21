use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use brim_core::models::ProviderId;
use brim_core::provider::{AuthSessionCommand, AuthSessionState, ProviderAuthSession};
use brim_providers::sync_engine::SyncEngine;
use brim_storage::config::AppConfig;
use brim_storage::db::Database;
use brim_storage::paths::AppPaths;

mod app;
mod ui;

use app::{App, AppAction};

fn build_engine(
    config: Arc<AppConfig>,
    db: Arc<Database>,
    http: Arc<reqwest::Client>,
) -> Arc<SyncEngine> {
    let registry = brim_providers::registry::build_registry(http, &config);
    Arc::new(SyncEngine::new(registry, db, config))
}

fn available_providers(config: &AppConfig) -> Vec<ProviderId> {
    ProviderId::all()
        .iter()
        .copied()
        .filter(|id| !config.provider(*id).enabled)
        .collect()
}

fn schedule_next_poll(app: &App, next_auth_poll_at: &mut Option<Instant>) {
    *next_auth_poll_at = app
        .modal
        .as_ref()
        .and_then(|modal| modal.view.poll_interval_secs)
        .filter(|_| {
            app.modal
                .as_ref()
                .map(|modal| !modal.status.is_terminal())
                .unwrap_or(false)
        })
        .map(|secs| Instant::now() + Duration::from_secs(secs));
}

fn start_auth_popup(
    app: &mut App,
    engine: &Arc<SyncEngine>,
    rt: &tokio::runtime::Runtime,
    provider: ProviderId,
    auth_session: &mut Option<Box<dyn ProviderAuthSession>>,
    next_auth_poll_at: &mut Option<Instant>,
) {
    match engine.registry().get(provider) {
        Some(p) => match p.begin_auth_session() {
            Ok(mut session) => match rt.block_on(session.advance(AuthSessionCommand::Start)) {
                Ok(state) => {
                    let view = session.view();
                    app.open_auth_modal(provider, view, state);
                    *auth_session = Some(session);
                    schedule_next_poll(app, next_auth_poll_at);
                }
                Err(err) => {
                    let view = session.view();
                    app.open_auth_modal(provider, view, AuthSessionState::Failed(err.to_string()));
                    *auth_session = Some(session);
                    *next_auth_poll_at = None;
                }
            },
            Err(err) => {
                app.message = format!("{} auth failed: {}", provider.display_name(), err);
            }
        },
        None => {
            app.message = format!("{} is not currently enabled", provider.display_name());
        }
    }
}

/// Run the interactive TUI. Caller supplies already-built shared state (the CLI
/// reuses its own; the standalone `brim-tui` binary builds it in `main`).
///
/// Builds its own tokio runtime, so it must NOT be called from inside an async
/// runtime — spawn it on a dedicated thread (see the `brim` CLI).
pub fn run(
    paths: AppPaths,
    config: Arc<AppConfig>,
    db: Arc<Database>,
    http: Arc<reqwest::Client>,
) -> Result<()> {
    let mut config = config;
    let mut engine = build_engine(config.clone(), db.clone(), http.clone());

    let rt = tokio::runtime::Runtime::new()?;
    let initial_statuses = rt.block_on(engine.all_statuses());
    let mut app = App::new(initial_statuses, available_providers(&config));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut auth_session: Option<Box<dyn ProviderAuthSession>> = None;
    let mut next_auth_poll_at: Option<Instant> = None;

    if !engine.registry().ids().is_empty() {
        app.set_syncing(true);
        terminal.draw(|f| ui::draw(f, &app))?;
        let statuses = rt.block_on(engine.fresh_statuses());
        app.update_statuses(statuses, available_providers(&config));
        app.set_syncing(false);
    }

    let tick_rate = Duration::from_millis(250);
    let refresh_interval = Duration::from_secs(60);
    let mut last_refresh = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if crossterm::event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                match app.handle_key(key) {
                    AppAction::Quit => break,
                    AppAction::Refresh => {
                        app.set_syncing(true);
                        terminal.draw(|f| ui::draw(f, &app))?;
                        let statuses = rt.block_on(engine.fresh_statuses());
                        app.update_statuses(statuses, available_providers(&config));
                        app.set_syncing(false);
                        last_refresh = Instant::now();
                    }
                    AppAction::OpenAddProvider => {
                        app.open_picker();
                    }
                    AppAction::OpenAuthPopup(provider) => {
                        start_auth_popup(
                            &mut app,
                            &engine,
                            &rt,
                            provider,
                            &mut auth_session,
                            &mut next_auth_poll_at,
                        );
                    }
                    AppAction::RemoveProvider(provider) => {
                        let mut new_config = (*config).clone();
                        new_config.set_provider_enabled(provider, false);
                        new_config
                            .save(&paths.config_file)
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        config = Arc::new(new_config);
                        engine = build_engine(config.clone(), db.clone(), http.clone());

                        let statuses = rt.block_on(engine.all_statuses());
                        app.update_statuses(statuses, available_providers(&config));
                        app.message = format!("{} removed", provider.display_name());
                        app.back_to_dashboard_or_picker();
                    }
                    AppAction::ConfirmSetup(provider) => {
                        let mut new_config = (*config).clone();
                        new_config.set_provider_enabled(provider, true);
                        new_config
                            .save(&paths.config_file)
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        config = Arc::new(new_config);
                        engine = build_engine(config.clone(), db.clone(), http.clone());

                        let statuses = rt.block_on(engine.all_statuses());
                        app.update_statuses(statuses, available_providers(&config));
                        app.back_to_dashboard_or_picker();

                        start_auth_popup(
                            &mut app,
                            &engine,
                            &rt,
                            provider,
                            &mut auth_session,
                            &mut next_auth_poll_at,
                        );
                        last_refresh = Instant::now();
                    }
                    AppAction::Back => {
                        app.back_to_dashboard_or_picker();
                    }
                    AppAction::AuthModalConfirm => {
                        if let Some(session) = auth_session.as_mut() {
                            match rt.block_on(session.advance(AuthSessionCommand::Confirm)) {
                                Ok(state) => {
                                    let view = session.view();
                                    app.update_auth_modal(view, state);
                                    schedule_next_poll(&app, &mut next_auth_poll_at);
                                }
                                Err(err) => {
                                    let view = session.view();
                                    app.update_auth_modal(
                                        view,
                                        AuthSessionState::Failed(err.to_string()),
                                    );
                                    next_auth_poll_at = None;
                                }
                            }
                        }
                    }
                    AppAction::AuthModalCancel => {
                        if let Some(session) = auth_session.as_mut() {
                            match rt.block_on(session.advance(AuthSessionCommand::Cancel)) {
                                Ok(state) => {
                                    let view = session.view();
                                    app.update_auth_modal(view, state);
                                    next_auth_poll_at = None;
                                }
                                Err(err) => {
                                    let view = session.view();
                                    app.update_auth_modal(
                                        view,
                                        AuthSessionState::Failed(err.to_string()),
                                    );
                                    next_auth_poll_at = None;
                                }
                            }
                        } else {
                            app.close_auth_modal();
                        }
                    }
                    AppAction::AuthModalClose => {
                        if let Some(modal) = app.close_auth_modal() {
                            auth_session = None;
                            next_auth_poll_at = None;
                            if modal.pending_refresh_after_close {
                                app.set_syncing(true);
                                terminal.draw(|f| ui::draw(f, &app))?;
                                let statuses = rt.block_on(engine.fresh_statuses());
                                app.update_statuses(statuses, available_providers(&config));
                                app.set_syncing(false);
                                app.message =
                                    format!("{} authenticated", modal.provider.display_name());
                                last_refresh = Instant::now();
                            } else {
                                app.message = modal.view.status_text;
                            }
                        }
                    }
                    AppAction::None => {}
                }
            }
        }

        if let Some(poll_at) = next_auth_poll_at {
            if Instant::now() >= poll_at {
                if let Some(session) = auth_session.as_mut() {
                    match rt.block_on(session.advance(AuthSessionCommand::Poll)) {
                        Ok(state) => {
                            let view = session.view();
                            app.update_auth_modal(view, state);
                            schedule_next_poll(&app, &mut next_auth_poll_at);
                        }
                        Err(err) => {
                            let view = session.view();
                            app.update_auth_modal(view, AuthSessionState::Failed(err.to_string()));
                            next_auth_poll_at = None;
                        }
                    }
                } else {
                    next_auth_poll_at = None;
                }
            }
        }

        if last_refresh.elapsed() >= refresh_interval && app.modal.is_none() {
            app.set_syncing(true);
            terminal.draw(|f| ui::draw(f, &app))?;
            let statuses = rt.block_on(engine.fresh_statuses());
            app.update_statuses(statuses, available_providers(&config));
            app.set_syncing(false);
            last_refresh = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
