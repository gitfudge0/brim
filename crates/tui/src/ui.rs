use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};
use ratatui::Frame;

use brim_core::confidence::Confidence;
use brim_core::models::{AuthState, PlanInfo, ProviderId, ProviderStatus};

use crate::app::{App, Screen};

/// Main draw function — renders the entire TUI frame.
pub fn draw(f: &mut Frame, app: &App) {
    let size = f.area();
    f.render_widget(Clear, size);

    // Layout: header (3 lines) | body (flex) | footer (3 lines)
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(10),   // body
            Constraint::Length(3), // footer
        ])
        .split(size);

    draw_header(f, outer[0], app);
    draw_body(f, outer[1], app);
    draw_footer(f, outer[2], app);

    if let Some(modal) = &app.modal {
        draw_auth_modal(f, size, modal);
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let sync_indicator = if app.syncing {
        Span::styled(" [syncing...] ", Style::default().fg(Color::Yellow))
    } else {
        Span::styled("", Style::default())
    };

    let title = Line::from(vec![
        Span::styled(
            format!(" {} ", brim_core::brand::GLYPH),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            brim_core::brand::NAME,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            concat!(" v", env!("CARGO_PKG_VERSION"), " "),
            Style::default().fg(Color::DarkGray),
        ),
        sync_indicator,
    ]);

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));

    let header = Paragraph::new(title).block(block);
    f.render_widget(header, area);
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    match app.screen {
        Screen::Dashboard => draw_dashboard(f, area, app),
        Screen::ProviderPicker => draw_provider_picker(f, area, app),
        Screen::ProviderSetup(provider) => draw_provider_setup(f, area, app, provider),
    }
}

fn draw_dashboard(f: &mut Frame, area: Rect, app: &App) {
    if app.statuses.is_empty() {
        let msg = Paragraph::new("No services added yet. Press 'a' to add one.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, area);
        return;
    }

    // Split body into equal columns for each provider
    let constraints: Vec<Constraint> = app
        .statuses
        .iter()
        .map(|_| Constraint::Ratio(1, app.statuses.len() as u32))
        .collect();

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, status) in app.statuses.iter().enumerate() {
        let is_selected = i == app.selected;
        draw_provider_panel(f, columns[i], status, is_selected);
    }
}

fn draw_provider_picker(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Add A Service ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .padding(Padding::new(1, 1, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![
        Line::from(Span::styled(
            "Choose the service you want to track.",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if app.available_providers.is_empty() {
        lines.push(Line::from(Span::styled(
            "All supported services are already added.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (idx, provider) in app.available_providers.iter().enumerate() {
            let is_selected = idx == app.picker_selected;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(
                format!("{}{}", prefix, provider.display_name()),
                style,
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter to continue, Esc to go back",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_provider_setup(f: &mut Frame, area: Rect, app: &App, provider: ProviderId) {
    let block = Block::default()
        .title(format!(" Set Up {} ", provider.display_name()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .padding(Padding::new(1, 1, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![];
    lines.push(Line::from(Span::styled(
        format!("Track {} usage", provider.display_name()),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for line in setup_instructions(provider) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(""));

    if let Some(status) = app
        .statuses
        .iter()
        .find(|status| status.provider == provider)
    {
        let (auth_label, auth_color) = match &status.auth_state {
            AuthState::Authenticated => ("authenticated", Color::Green),
            AuthState::Configured => ("configured", Color::Yellow),
            AuthState::Expired => ("expired", Color::Red),
            AuthState::Failed(_) => ("failed", Color::Red),
            AuthState::NotConfigured => ("not configured", Color::DarkGray),
        };
        lines.push(Line::from(vec![
            Span::styled("Current status: ", Style::default().fg(Color::DarkGray)),
            Span::styled(auth_label, Style::default().fg(auth_color)),
        ]));
        lines.push(Line::from(""));
    } else {
        lines.push(Line::from(Span::styled(
            "Current status: not added yet",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
    }

    let confirm_text = match provider {
        ProviderId::Copilot => "Press Enter to open the GitHub auth popup.",
        ProviderId::Codex | ProviderId::Claude => "Press Enter to open the auth popup.",
    };
    lines.push(Line::from(Span::styled(
        confirm_text,
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "Esc to go back",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_provider_panel(f: &mut Frame, area: Rect, status: &ProviderStatus, selected: bool) {
    let provider_name = status.provider.display_name();
    let border_color = if selected {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(format!(" {} ", provider_name))
        .title_style(
            Style::default()
                .fg(if selected { Color::Cyan } else { Color::White })
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .padding(Padding::new(1, 1, 0, 0));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Build the content lines for this provider panel
    let mut lines: Vec<Line> = Vec::new();

    // Auth state
    let (auth_label, auth_color) = match &status.auth_state {
        AuthState::Authenticated => ("authenticated", Color::Green),
        AuthState::Configured => ("configured", Color::Yellow),
        AuthState::Expired => ("expired", Color::Red),
        AuthState::Failed(_msg) => ("failed", Color::Red),
        AuthState::NotConfigured => ("not configured", Color::DarkGray),
    };
    lines.push(Line::from(vec![
        Span::styled("Auth: ", Style::default().fg(Color::DarkGray)),
        Span::styled(auth_label, Style::default().fg(auth_color)),
    ]));

    if let AuthState::Failed(msg) = &status.auth_state {
        lines.push(Line::from(Span::styled(
            format!("Reason: {}", truncate(msg, inner.width as usize - 10)),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(Span::styled(
            "Press 'e' to re-authenticate",
            Style::default().fg(Color::Yellow),
        )));
    } else if matches!(status.auth_state, AuthState::Expired) {
        lines.push(Line::from(Span::styled(
            "Press 'e' to re-authenticate",
            Style::default().fg(Color::Yellow),
        )));
    }

    let setup_badge = if status.auth_state.is_usable() {
        ("ready", Color::Green)
    } else {
        ("setup needed", Color::Yellow)
    };
    lines.push(Line::from(vec![
        Span::styled("Setup: ", Style::default().fg(Color::DarkGray)),
        Span::styled(setup_badge.0, Style::default().fg(setup_badge.1)),
    ]));

    // Plan info
    if let Some(ref snapshot) = status.last_snapshot {
        if let Some(ref plan) = snapshot.plan {
            lines.push(Line::from(plan_spans(plan)));
        }

        // Source + age
        let age = format_age(snapshot.fetched_at);
        lines.push(Line::from(vec![
            Span::styled("Source: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &snapshot.source_strategy,
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!(" ({})", age), Style::default().fg(Color::DarkGray)),
        ]));

        lines.push(Line::from(""));

        // Quota buckets
        if snapshot.buckets.is_empty() {
            lines.push(Line::from(Span::styled(
                "No quota data",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for bucket in &snapshot.buckets {
                // Bucket label
                lines.push(Line::from(Span::styled(
                    &bucket.label,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )));

                // Percentage bar
                if let Some(pct) = bucket.effective_percent_remaining() {
                    let bar_str = render_bar(pct.value, 20);
                    let bar_color = pct_color(pct.value);
                    let conf_color = confidence_color(pct.confidence);
                    let pct_str = format!(" {:.0}%", pct.value * 100.0);
                    let conf_str = if pct.confidence.needs_warning() {
                        format!(" [{}]", pct.confidence)
                    } else {
                        String::new()
                    };

                    lines.push(Line::from(vec![
                        Span::styled(bar_str, Style::default().fg(bar_color)),
                        Span::styled(
                            pct_str,
                            Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(conf_str, Style::default().fg(conf_color)),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(
                        "  (no data)",
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                // Reset time
                if let Some(remaining) = bucket.window.time_remaining() {
                    let hours = remaining.num_hours();
                    let reset_text = if hours >= 24 {
                        if let Some(dt) = bucket.window.resets_at {
                            let local = dt.with_timezone(&chrono::Local);
                            format!("  Resets: {}", local.format("%b %-d %-I%p"))
                        } else {
                            format!("  Resets in {}h {}m", hours, remaining.num_minutes() % 60)
                        }
                    } else {
                        let mins = remaining.num_minutes() % 60;
                        format!("  Resets in {}h {}m", hours, mins)
                    };
                    lines.push(Line::from(Span::styled(
                        reset_text,
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                lines.push(Line::from(""));
            }
        }

        // Notes
        for note in &snapshot.notes {
            lines.push(Line::from(Span::styled(
                format!("* {}", truncate(note, inner.width as usize - 4)),
                Style::default().fg(Color::DarkGray),
            )));
        }

        if matches!(status.auth_state, AuthState::Failed(_)) {
            lines.push(Line::from(Span::styled(
                "* cached data shown; refresh failed due to authentication",
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        lines.push(Line::from(""));
        if status.auth_state.is_usable() {
            lines.push(Line::from(Span::styled(
                "No cached data",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "Press 'r' to sync",
                Style::default().fg(Color::Yellow),
            )));
        } else if matches!(status.auth_state, AuthState::Failed(_) | AuthState::Expired) {
            lines.push(Line::from(Span::styled(
                "Authentication needs attention",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "Press 'e' to re-authenticate",
                Style::default().fg(Color::Yellow),
            )));
            lines.push(Line::from(Span::styled(
                format!("brim auth login {}", status.provider.as_str()),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "Not configured",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                format!("brim auth login {}", status.provider.as_str()),
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    let content = Paragraph::new(lines);
    f.render_widget(content, inner);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));

    let keybinds = match app.screen {
        _ if app.modal.is_some() => Line::from(vec![
            Span::styled(
                " Enter",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" confirm/close  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cancel/close  ", Style::default().fg(Color::DarkGray)),
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled("Auth popup active", Style::default().fg(Color::DarkGray)),
        ]),
        Screen::Dashboard => Line::from(vec![
            Span::styled(
                " q",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" quit  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "a",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" add service  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "x",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" remove service  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "r",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" refresh  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "e",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" re-auth  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Tab",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" navigate  ", Style::default().fg(Color::DarkGray)),
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.message, Style::default().fg(Color::DarkGray)),
        ]),
        Screen::ProviderPicker => Line::from(vec![
            Span::styled(
                " Enter",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" continue  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.message, Style::default().fg(Color::DarkGray)),
        ]),
        Screen::ProviderSetup(_) => Line::from(vec![
            Span::styled(
                " Enter",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" continue  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.message, Style::default().fg(Color::DarkGray)),
        ]),
    };

    let footer = Paragraph::new(keybinds).block(block);
    f.render_widget(footer, area);
}

// --- Helpers ---

fn render_bar(fraction: f64, width: usize) -> String {
    let filled = (fraction * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

fn draw_auth_modal(f: &mut Frame, area: Rect, modal: &crate::app::AuthModalState) {
    let popup = centered_rect(60, 55, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(format!(" {} ", modal.view.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .padding(Padding::new(1, 1, 1, 1));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines = vec![Line::from(Span::styled(
        modal.provider.display_name(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];

    if let Some(subtitle) = &modal.view.subtitle {
        lines.push(Line::from(Span::styled(
            subtitle.clone(),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));

    if let Some(uri) = &modal.view.verification_uri {
        lines.push(Line::from(vec![
            Span::styled("Open: ", Style::default().fg(Color::DarkGray)),
            Span::styled(uri.clone(), Style::default().fg(Color::White)),
        ]));
    }

    if let Some(code) = &modal.view.user_code {
        lines.push(Line::from(vec![
            Span::styled("Code: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                code.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    if modal.view.verification_uri.is_some() || modal.view.user_code.is_some() {
        lines.push(Line::from(""));
    }

    for help in &modal.view.help_text {
        lines.push(Line::from(Span::styled(
            truncate(help, inner.width as usize - 2),
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(""));

    let status_color = match AuthStateOrSession::from(&modal.status) {
        AuthStateOrSession::Succeeded => Color::Green,
        AuthStateOrSession::Failed => Color::Red,
        AuthStateOrSession::Cancelled => Color::DarkGray,
        AuthStateOrSession::Active => Color::Yellow,
    };
    lines.push(Line::from(vec![
        Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            truncate(&modal.view.status_text, inner.width as usize - 10),
            Style::default().fg(status_color),
        ),
    ]));

    lines.push(Line::from(""));

    let confirm_label = modal
        .view
        .confirm_label
        .clone()
        .unwrap_or_else(|| "Confirm".into());
    let action_text = match &modal.status {
        brim_core::provider::AuthSessionState::Succeeded(_) => {
            format!("Enter {}  Esc close", confirm_label)
        }
        brim_core::provider::AuthSessionState::Failed(_) => {
            format!("Enter {}  Esc close", confirm_label)
        }
        brim_core::provider::AuthSessionState::Cancelled => "Enter close  Esc close".into(),
        _ => format!("Enter {}  Esc cancel", confirm_label),
    };
    lines.push(Line::from(Span::styled(
        action_text,
        Style::default().fg(Color::Cyan),
    )));

    f.render_widget(Paragraph::new(lines), inner);
}

enum AuthStateOrSession {
    Active,
    Succeeded,
    Failed,
    Cancelled,
}

impl From<&brim_core::provider::AuthSessionState> for AuthStateOrSession {
    fn from(value: &brim_core::provider::AuthSessionState) -> Self {
        match value {
            brim_core::provider::AuthSessionState::Succeeded(_) => Self::Succeeded,
            brim_core::provider::AuthSessionState::Failed(_) => Self::Failed,
            brim_core::provider::AuthSessionState::Cancelled => Self::Cancelled,
            _ => Self::Active,
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn pct_color(pct: f64) -> Color {
    if pct >= 0.5 {
        Color::Green
    } else if pct >= 0.2 {
        Color::Yellow
    } else if pct > 0.0 {
        Color::Red
    } else {
        Color::DarkGray
    }
}

fn confidence_color(conf: Confidence) -> Color {
    match conf {
        Confidence::Official => Color::Green,
        Confidence::ProviderLocal => Color::Blue,
        Confidence::Experimental => Color::Yellow,
        Confidence::Derived => Color::Magenta,
        Confidence::Stale => Color::Red,
    }
}

fn plan_spans(plan: &PlanInfo) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled("Plan: ", Style::default().fg(Color::DarkGray))];
    spans.push(Span::styled(
        plan.name.value.clone(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));

    if plan.tier.is_none() {
        spans.push(Span::styled(
            format!(" [{}]", plan.name.confidence),
            Style::default().fg(confidence_color(plan.name.confidence)),
        ));
        return spans;
    }

    if plan.display_text()
        == format!(
            "{} / {} [{}]",
            plan.name.value,
            plan.tier
                .as_ref()
                .map(|tier| tier.value.as_str())
                .unwrap_or_default(),
            plan.name.confidence
        )
    {
        let tier = plan.tier.as_ref().expect("tier should exist");
        spans.push(Span::styled(
            format!(" / {}", tier.value),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" [{}]", plan.name.confidence),
            Style::default().fg(confidence_color(plan.name.confidence)),
        ));
        return spans;
    }

    let tier = plan.tier.as_ref().expect("tier should exist");
    spans.push(Span::styled(
        format!(" [{}]", plan.name.confidence),
        Style::default().fg(confidence_color(plan.name.confidence)),
    ));
    spans.push(Span::styled(
        " / ".to_string(),
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::styled(
        tier.value.clone(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!(" [{}]", tier.confidence),
        Style::default().fg(confidence_color(tier.confidence)),
    ));

    spans
}

fn format_age(dt: chrono::DateTime<chrono::Utc>) -> String {
    let age = chrono::Utc::now() - dt;
    if age.num_seconds() < 60 {
        "just now".into()
    } else if age.num_minutes() < 60 {
        format!("{}m ago", age.num_minutes())
    } else if age.num_hours() < 24 {
        format!("{}h ago", age.num_hours())
    } else {
        format!("{}d ago", age.num_days())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max > 3 {
        format!("{}...", &s[..max - 3])
    } else {
        s[..max].to_string()
    }
}

fn setup_instructions(provider: ProviderId) -> Vec<&'static str> {
    match provider {
        ProviderId::Codex => vec![
            "1. Install and sign in with the `codex` CLI.",
            "2. Run `codex` in a terminal and complete login.",
            "3. This app reads usage from `codex app-server` and local auth state.",
        ],
        ProviderId::Claude => vec![
            "1. Install and sign in with the `claude` CLI.",
            "2. Run `claude` in a terminal and complete login.",
            "3. This app reads local Claude credentials and, when available, the OAuth usage API.",
        ],
        ProviderId::Copilot => vec![
            "1. Sign in with your GitHub account to access Copilot usage.",
            "2. The app can start GitHub device login for you.",
            "3. Usage comes from GitHub's Copilot internal endpoint.",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::plan_spans;
    use brim_core::confidence::Labeled;
    use brim_core::models::PlanInfo;

    fn spans_to_string(spans: Vec<ratatui::text::Span<'static>>) -> String {
        spans
            .into_iter()
            .map(|span| span.content.to_string())
            .collect()
    }

    #[test]
    fn renders_plan_name_only() {
        let plan = PlanInfo {
            name: Labeled::provider_local("Claude".to_string()),
            tier: None,
        };

        assert_eq!(spans_to_string(plan_spans(&plan)), "Plan: Claude [local]");
    }

    #[test]
    fn renders_plan_and_tier_with_distinct_confidences() {
        let plan = PlanInfo {
            name: Labeled::experimental("Claude".to_string()),
            tier: Some(Labeled::provider_local("pro".to_string())),
        };

        assert_eq!(
            spans_to_string(plan_spans(&plan)),
            "Plan: Claude [experimental] / pro [local]"
        );
    }

    #[test]
    fn renders_plan_and_tier_with_same_confidence() {
        let plan = PlanInfo {
            name: Labeled::provider_local("Claude".to_string()),
            tier: Some(Labeled::provider_local("pro".to_string())),
        };

        assert_eq!(
            spans_to_string(plan_spans(&plan)),
            "Plan: Claude / pro [local]"
        );
    }
}
