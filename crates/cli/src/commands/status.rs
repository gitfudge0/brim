use anyhow::Result;
use brim_core::history::compute_history_metrics;
use brim_core::models::{AuthState, ProviderId, ProviderStatus};
use brim_providers::sync_engine::SyncEngine;

pub async fn run(engine: &SyncEngine, provider: Option<String>, fresh: bool) -> Result<()> {
    let show_all_supported = provider.is_none();
    let statuses = collect_supported_statuses(engine, provider, fresh).await?;

    if show_all_supported && engine.config().enabled_provider_ids().is_empty() {
        println!("{}", crate::commands::no_enabled_providers_message());
        println!();
    }

    for status in &statuses {
        // Compute burn rate from the last 1 day of history (best-effort).
        let burn_info = engine
            .db()
            .snapshots_for_history(status.provider, 1)
            .ok()
            .map(|snaps| {
                let m = compute_history_metrics(&snaps);
                (m.burn_rate, m.time_to_empty_mins)
            });

        print_text_status(
            status.provider,
            &status.auth_state,
            status.last_snapshot.as_ref(),
            status.enabled,
            show_all_supported,
            burn_info.as_ref().and_then(|(br, _)| br.as_ref()),
            burn_info.as_ref().and_then(|(_, tte)| *tte),
        );
    }

    Ok(())
}

pub async fn collect_statuses(
    engine: &SyncEngine,
    provider: Option<String>,
    fresh: bool,
) -> Result<Vec<ProviderStatus>> {
    let ids = match provider {
        Some(name) => vec![crate::commands::parse_provider_arg(&name)?],
        None => engine.config().enabled_provider_ids(),
    };
    let mut statuses = Vec::with_capacity(ids.len());

    for id in ids {
        if fresh {
            statuses.push(engine.fresh_status(id).await);
            continue;
        }

        let snapshot = engine.cached_snapshot(id);
        let auth = match engine.registry().get(id) {
            Some(p) => p.auth_state().await,
            None => AuthState::NotConfigured,
        };

        statuses.push(ProviderStatus {
            provider: id,
            auth_state: auth,
            last_snapshot: snapshot,
            enabled: engine.registry().get(id).is_some(),
        });
    }

    Ok(statuses)
}

pub async fn collect_supported_statuses(
    engine: &SyncEngine,
    provider: Option<String>,
    fresh: bool,
) -> Result<Vec<ProviderStatus>> {
    let explicit_provider = provider.is_some();
    let ids = match provider {
        Some(name) => vec![crate::commands::parse_provider_arg(&name)?],
        None => ProviderId::all().to_vec(),
    };
    let mut statuses = Vec::with_capacity(ids.len());

    for id in ids {
        let should_fetch_fresh =
            fresh && (explicit_provider || engine.config().provider(id).enabled);
        if should_fetch_fresh {
            statuses.push(engine.fresh_status(id).await);
            continue;
        }

        let snapshot = engine.cached_snapshot(id);
        let auth = match engine.registry().get(id) {
            Some(p) => p.auth_state().await,
            None => AuthState::NotConfigured,
        };

        statuses.push(ProviderStatus {
            provider: id,
            auth_state: auth,
            last_snapshot: snapshot,
            enabled: engine.config().provider(id).enabled,
        });
    }

    Ok(statuses)
}

fn print_text_status(
    id: ProviderId,
    auth: &brim_core::models::AuthState,
    snapshot: Option<&brim_core::models::UsageSnapshot>,
    enabled: bool,
    show_enabled: bool,
    burn_rate: Option<&brim_core::history::BurnRate>,
    time_to_empty_mins: Option<f64>,
) {
    println!("--- {} ---", id.display_name());
    if show_enabled {
        println!("  Enabled: {}", if enabled { "yes" } else { "no" });
    }
    println!("  Auth: {}", auth);

    match snapshot {
        Some(snap) => {
            if let Some(plan) = &snap.plan {
                println!("  Plan: {}", plan.display_text());
            }
            println!(
                "  Source: {} (fetched {})",
                snap.source_strategy,
                format_age(snap.fetched_at)
            );

            if snap.buckets.is_empty() {
                println!("  No quota data available");
            }

            for bucket in &snap.buckets {
                let pct_str = bucket
                    .effective_percent_remaining()
                    .map(|p| {
                        let bar = progress_bar(p.value, 20);
                        let warning = if p.confidence.needs_warning() {
                            format!(" [{}]", p.confidence)
                        } else {
                            String::new()
                        };
                        format!("{} {:.0}%{}", bar, p.value * 100.0, warning)
                    })
                    .unwrap_or_else(|| "unknown".into());

                println!("  {}: {}", bucket.label, pct_str);

                if let Some(remaining) = bucket.window.time_remaining() {
                    let hours = remaining.num_hours();
                    if hours >= 24 {
                        // Show absolute date when > 24h away
                        if let Some(dt) = bucket.window.resets_at {
                            let local = dt.with_timezone(&chrono::Local);
                            println!("    Resets: {}", local.format("%b %-d %-I%p"));
                        } else {
                            println!(
                                "    Resets in: {}h {}m",
                                hours,
                                remaining.num_minutes() % 60
                            );
                        }
                    } else {
                        let mins = remaining.num_minutes() % 60;
                        println!("    Resets in: {}h {}m", hours, mins);
                    }
                }
            }

            // Burn rate inline (if available from recent history)
            if let Some(br) = burn_rate {
                let line = crate::commands::history::burn_rate_line(br, time_to_empty_mins);
                println!("  {}", line);
            }

            for note in &snap.notes {
                println!("  Note: {}", note);
            }

            if snap
                .notes
                .iter()
                .any(|note| note.to_lowercase().contains("stale"))
            {
                println!("  Hint: Run `brim sync {}` for fresh data.", id.as_str());
            }

            if let Some(note) = auth_cached_data_note(auth, snapshot.is_some()) {
                println!("  Note: {}", note);
            }
        }
        None => {
            if !enabled {
                println!("  Disabled in config");
            } else if auth.is_usable() {
                println!("  No cached data. Run `brim sync` to fetch.");
            } else {
                println!(
                    "  Not configured. Run `brim auth login {}` to set up.",
                    id.as_str()
                );
            }
        }
    }

    if let Some(hint) = reauth_hint(auth, id) {
        println!("  {}", hint);
    }

    println!();
}

fn auth_cached_data_note(auth: &AuthState, has_snapshot: bool) -> Option<&'static str> {
    if has_snapshot && matches!(auth, AuthState::Failed(_)) {
        Some("cached data shown; refresh failed due to authentication")
    } else {
        None
    }
}

fn reauth_hint(auth: &AuthState, id: ProviderId) -> Option<String> {
    if matches!(auth, AuthState::Failed(_) | AuthState::Expired) {
        Some(format!(
            "Run `brim auth login {}` to re-authenticate.",
            id.as_str()
        ))
    } else {
        None
    }
}

fn progress_bar(fraction: f64, width: usize) -> String {
    let filled = (fraction * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
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

#[cfg(test)]
mod tests {
    use brim_core::confidence::Labeled;
    use brim_core::models::{AuthState, PlanInfo, ProviderId};

    #[test]
    fn formats_plan_name_only() {
        let plan = PlanInfo {
            name: Labeled::provider_local("Claude".to_string()),
            tier: None,
        };

        assert_eq!(plan.display_text(), "Claude [local]");
    }

    #[test]
    fn formats_plan_and_tier_with_same_confidence() {
        let plan = PlanInfo {
            name: Labeled::provider_local("Claude".to_string()),
            tier: Some(Labeled::provider_local("pro".to_string())),
        };

        assert_eq!(plan.display_text(), "Claude / pro [local]");
    }

    #[test]
    fn formats_plan_and_tier_with_different_confidence() {
        let plan = PlanInfo {
            name: Labeled::experimental("Claude".to_string()),
            tier: Some(Labeled::provider_local("pro".to_string())),
        };

        assert_eq!(plan.display_text(), "Claude [experimental] / pro [local]");
    }

    #[test]
    fn reauth_hint_is_shown_for_failed_auth() {
        let hint = super::reauth_hint(
            &AuthState::Failed("token rejected".into()),
            ProviderId::Copilot,
        );

        assert_eq!(
            hint.as_deref(),
            Some("Run `brim auth login copilot` to re-authenticate.")
        );
    }

    #[test]
    fn cached_data_note_only_for_failed_auth_with_snapshot() {
        assert_eq!(
            super::auth_cached_data_note(&AuthState::Failed("bad token".into()), true),
            Some("cached data shown; refresh failed due to authentication")
        );
        assert_eq!(
            super::auth_cached_data_note(&AuthState::Configured, true),
            None
        );
        assert_eq!(
            super::auth_cached_data_note(&AuthState::Failed("bad token".into()), false),
            None
        );
    }

    #[test]
    fn cached_data_stale_hint_is_detected_by_note() {
        let notes = ["Data is stale (older than TTL)"];
        assert!(notes
            .iter()
            .any(|note| note.to_lowercase().contains("stale")));
    }
}
