//! `brim history` — show historical usage, burn rate, and weekly trends.

use anyhow::Result;
use brim_core::history::{compute_history_metrics, BurnRate, HistoryMetrics};
use brim_core::models::ProviderId;
use brim_core::time_window::WindowKind;
use brim_providers::sync_engine::SyncEngine;

pub async fn run(
    engine: &SyncEngine,
    provider: Option<String>,
    days: u32,
    json: bool,
) -> Result<()> {
    let ids: Vec<ProviderId> = match provider {
        Some(ref name) => vec![crate::commands::parse_provider_arg(name)?],
        None => ProviderId::all().to_vec(),
    };

    for id in ids {
        let snapshots = engine
            .db()
            .snapshots_for_history(id, days)
            .map_err(|e| anyhow::anyhow!("db error: {}", e))?;

        let metrics = compute_history_metrics(&snapshots);

        if json {
            let out = serde_json::json!({
                "provider": id.as_str(),
                "days": days,
                "metrics": metrics,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            print_history(id, days, &metrics, &snapshots);
        }
    }

    Ok(())
}

fn print_history(
    id: ProviderId,
    days: u32,
    metrics: &HistoryMetrics,
    snapshots: &[brim_core::models::UsageSnapshot],
) {
    // Header
    println!("--- {}  ·  last {} days ---", id.display_name(), days);

    // Plan info from latest snapshot
    if let Some(snap) = snapshots.last() {
        if let Some(plan) = &snap.plan {
            println!("Plan: {}", plan.display_text());
        }
    }

    // Weekly sparkline
    if !metrics.weekly_peaks.is_empty() {
        let sparkline = build_sparkline(&metrics.weekly_peaks);
        println!("\nWeekly usage");
        println!("  {}  {:.0}% peak", sparkline, weekly_latest_pct(metrics));
    }

    // Session bucket from latest snapshot
    if let Some(snap) = snapshots.last() {
        if let Some(bucket) = snap
            .buckets
            .iter()
            .find(|b| b.window.kind == WindowKind::Session)
        {
            println!("\nSession ({})", bucket.window.label);
            if let Some(pr) = bucket.effective_percent_remaining() {
                let bar = progress_bar_10(pr.value);
                println!("  {}  {:.0}% remaining", bar, pr.value * 100.0);
            }
            if let Some(remaining) = bucket.window.time_remaining() {
                let h = remaining.num_hours();
                let m = remaining.num_minutes() % 60;
                println!("  resets in: {}h {}m", h, m);
            }
        }
    }

    // Burn rate line
    match &metrics.burn_rate {
        Some(br) => {
            let stale_label = if br.is_stale { "  [stale]" } else { "" };
            let pace_str = match metrics.time_to_empty_mins {
                Some(mins) => format!("  ·  ~{} at this pace", format_mins(mins)),
                None => String::new(),
            };
            println!(
                "  burn: {:.1} req/hr{}{}",
                br.req_per_hour, pace_str, stale_label
            );
        }
        None => {
            println!("  burn: insufficient data");
        }
    }

    // 4-week history table
    if !metrics.weekly_peaks.is_empty() {
        println!("\n{}-week history", metrics.weekly_peaks.len());
        let n = metrics.weekly_peaks.len();
        for (i, &peak) in metrics.weekly_peaks.iter().enumerate() {
            let offset = n as isize - 1 - i as isize;
            let week_label = if offset == 0 {
                "this wk".to_string()
            } else {
                format!("week -{}", offset)
            };
            let arrow = if offset == 0 { " ←" } else { "" };
            let bar = progress_bar_10(peak);
            println!("  {}  {}  {:.0}%{}", week_label, bar, peak * 100.0, arrow);
        }
    }

    // Session depth stats
    if let (Some(med), Some(p90)) = (metrics.session_depth_median, metrics.session_depth_p90) {
        println!("\nSession depth (across history)");
        println!("  median: {:.0}%  p90: {:.0}%", med * 100.0, p90 * 100.0);
    }

    println!();
}

fn weekly_latest_pct(metrics: &HistoryMetrics) -> f64 {
    metrics
        .weekly_peaks
        .last()
        .copied()
        .unwrap_or(0.0)
        * 100.0
}

/// Build a sparkline string from fractions 0.0–1.0 using 8-level Unicode blocks.
fn build_sparkline(values: &[f64]) -> String {
    const CHARS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    values
        .iter()
        .map(|&v| {
            let idx = ((v * 8.0).round() as usize).min(8);
            CHARS[idx]
        })
        .collect()
}

/// 10-char wide progress bar.
fn progress_bar_10(fraction: f64) -> String {
    let filled = ((fraction * 10.0).round() as usize).min(10);
    let empty = 10 - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn format_mins(mins: f64) -> String {
    if mins < 60.0 {
        format!("{:.0}m", mins)
    } else {
        let h = (mins / 60.0).floor() as u64;
        let m = (mins % 60.0).round() as u64;
        format!("{}h {}m", h, m)
    }
}

/// Render burn rate as a single display line (used by status command).
pub fn burn_rate_line(br: &BurnRate, time_to_empty_mins: Option<f64>) -> String {
    let stale_label = if br.is_stale { "  [stale]" } else { "" };
    let pace_str = match time_to_empty_mins {
        Some(mins) => format!("  ·  ~{} at this pace", format_mins(mins)),
        None => String::new(),
    };
    format!("burn: {:.1} req/hr{}{}", br.req_per_hour, pace_str, stale_label)
}
