//! History metrics engine: pure functions over `&[UsageSnapshot]`.
//!
//! No I/O; accepts slices of snapshots ordered ascending by `fetched_at`.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::models::UsageSnapshot;
use crate::time_window::WindowKind;

/// Burn rate derived from consecutive session snapshot deltas.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BurnRate {
    /// Average requests consumed per hour in the current/most-recent session.
    pub req_per_hour: f64,
    /// True when the newest snapshot is older than 2 hours.
    pub is_stale: bool,
}

/// All computed history/analytics metrics for one provider's snapshot history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HistoryMetrics {
    /// Estimated current burn rate (None when insufficient data).
    pub burn_rate: Option<BurnRate>,
    /// Minutes until the session bucket is exhausted at current burn rate,
    /// capped at the bucket's `resets_at` duration when available.
    pub time_to_empty_mins: Option<f64>,
    /// Max session `used/limit` fraction per ISO week, oldest → newest, up to 8 weeks.
    pub weekly_peaks: Vec<f64>,
    /// Median peak session usage fraction across contiguous sessions.
    pub session_depth_median: Option<f64>,
    /// 90th-percentile peak session usage fraction across contiguous sessions.
    pub session_depth_p90: Option<f64>,
}

/// Compute all history metrics from a slice of snapshots (ascending `fetched_at`).
pub fn compute_history_metrics(snapshots: &[UsageSnapshot]) -> HistoryMetrics {
    let burn_rate = compute_burn_rate(snapshots);
    let time_to_empty_mins = compute_time_to_empty(snapshots, burn_rate.as_ref());
    let weekly_peaks = compute_weekly_peaks(snapshots);
    let (session_depth_median, session_depth_p90) = compute_session_depths(snapshots);

    HistoryMetrics {
        burn_rate,
        time_to_empty_mins,
        weekly_peaks,
        session_depth_median,
        session_depth_p90,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Find the first session bucket in a snapshot, if any.
fn session_bucket(snap: &UsageSnapshot) -> Option<&crate::models::QuotaBucket> {
    snap.buckets
        .iter()
        .find(|b| b.window.kind == WindowKind::Session)
}

fn compute_burn_rate(snapshots: &[UsageSnapshot]) -> Option<BurnRate> {
    // Collect (fetched_at, used) pairs from snapshots that have a session bucket.
    let points: Vec<(chrono::DateTime<Utc>, f64)> = snapshots
        .iter()
        .filter_map(|s| {
            session_bucket(s)
                .and_then(|b| b.used.as_ref())
                .map(|u| (s.fetched_at, u.value))
        })
        .collect();

    if points.len() < 2 {
        return None;
    }

    // Restrict to the most recent contiguous session: find the last reset boundary
    // (where used decreased) and only use points from there onwards.
    let mut session_start = 0;
    for i in 1..points.len() {
        if points[i].1 < points[i - 1].1 {
            session_start = i;
        }
    }
    let session_points = &points[session_start..];
    if session_points.len() < 2 {
        return None;
    }

    let mut deltas: Vec<f64> = Vec::new();
    for window in session_points.windows(2) {
        let (t0, u0) = window[0];
        let (t1, u1) = window[1];
        let delta_used = u1 - u0;
        if delta_used <= 0.0 {
            continue;
        }
        let delta_hours = (t1 - t0).num_seconds() as f64 / 3600.0;
        if delta_hours <= 0.0 {
            continue;
        }
        deltas.push(delta_used / delta_hours);
    }

    if deltas.is_empty() {
        return None;
    }

    let req_per_hour = deltas.iter().sum::<f64>() / deltas.len() as f64;
    let newest_at = snapshots.last()?.fetched_at;
    let age_hours = (Utc::now() - newest_at).num_seconds() as f64 / 3600.0;
    let is_stale = age_hours > 2.0;

    Some(BurnRate {
        req_per_hour,
        is_stale,
    })
}

fn compute_time_to_empty(snapshots: &[UsageSnapshot], burn_rate: Option<&BurnRate>) -> Option<f64> {
    let burn = burn_rate?;
    if burn.req_per_hour <= 0.0 {
        return None;
    }

    // Use the most recent snapshot's session bucket.
    let latest = snapshots.last()?;
    let bucket = session_bucket(latest)?;

    // We need remaining amount. Try used + limit, else skip.
    let remaining = match (&bucket.used, &bucket.limit) {
        (Some(u), Some(l)) if l.value > 0.0 => l.value - u.value,
        _ => return None,
    };

    if remaining <= 0.0 {
        return Some(0.0);
    }

    let time_to_empty_hours = remaining / burn.req_per_hour;
    let mut time_to_empty_mins = time_to_empty_hours * 60.0;

    // Cap at the session bucket's reset time if available.
    if let Some(resets_at) = bucket.window.resets_at {
        let mins_to_reset = (resets_at - Utc::now()).num_seconds() as f64 / 60.0;
        if mins_to_reset > 0.0 {
            time_to_empty_mins = time_to_empty_mins.min(mins_to_reset);
        }
    }

    Some(time_to_empty_mins)
}

fn compute_weekly_peaks(snapshots: &[UsageSnapshot]) -> Vec<f64> {
    use chrono::Datelike;
    use std::collections::BTreeMap;

    // Map ISO (year, week) → max session used fraction seen that week.
    let mut week_peaks: BTreeMap<(i32, u32), f64> = BTreeMap::new();

    for snap in snapshots {
        let bucket = match session_bucket(snap) {
            Some(b) => b,
            None => continue,
        };
        let used = match &bucket.used {
            Some(u) => u.value,
            None => continue,
        };
        let limit = match &bucket.limit {
            Some(l) if l.value > 0.0 => l.value,
            _ => continue,
        };
        let fraction = (used / limit).clamp(0.0, 1.0);
        let iso = snap.fetched_at.iso_week();
        let key = (iso.year(), iso.week());
        let entry = week_peaks.entry(key).or_insert(0.0);
        if fraction > *entry {
            *entry = fraction;
        }
    }

    // Collect sorted and take last 8 weeks.
    let all: Vec<f64> = week_peaks.into_values().collect();
    let start = all.len().saturating_sub(8);
    all[start..].to_vec()
}

fn compute_session_depths(snapshots: &[UsageSnapshot]) -> (Option<f64>, Option<f64>) {
    // Group by contiguous sessions: a new session starts when resets_at changes
    // or when used decreases (session reset). Track the peak used-fraction per session.
    let mut session_peaks: Vec<f64> = Vec::new();
    let mut current_peak: f64 = 0.0;
    let mut prev_resets_at: Option<chrono::DateTime<Utc>> = None;
    let mut prev_used: Option<f64> = None;
    let mut in_session = false;

    for snap in snapshots {
        let bucket = match session_bucket(snap) {
            Some(b) => b,
            None => continue,
        };
        let used = match &bucket.used {
            Some(u) => u.value,
            None => continue,
        };
        let limit = match &bucket.limit {
            Some(l) if l.value > 0.0 => l.value,
            _ => continue,
        };
        let fraction = (used / limit).clamp(0.0, 1.0);
        let resets_at = bucket.window.resets_at;

        // Detect session boundary: resets_at changed or used decreased.
        let session_reset = match (prev_resets_at, resets_at) {
            (Some(prev), Some(curr)) => prev != curr,
            _ => false,
        } || prev_used.map(|p| used < p).unwrap_or(false);

        if session_reset && in_session {
            session_peaks.push(current_peak);
            current_peak = 0.0;
        }

        if fraction > current_peak {
            current_peak = fraction;
        }
        in_session = true;
        prev_resets_at = resets_at;
        prev_used = Some(used);
    }

    // Don't forget the last (possibly still-open) session.
    if in_session {
        session_peaks.push(current_peak);
    }

    if session_peaks.is_empty() {
        return (None, None);
    }

    let median = percentile(&mut session_peaks.clone(), 50.0);
    let p90 = percentile(&mut session_peaks.clone(), 90.0);
    (Some(median), Some(p90))
}

/// Compute a percentile (0–100) of a slice; sorts the data in place.
fn percentile(data: &mut [f64], p: f64) -> f64 {
    data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if data.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (data.len() - 1) as f64).round() as usize;
    data[idx.min(data.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confidence::Labeled;
    use crate::models::{ProviderId, QuotaBucket, UsageSnapshot};
    use crate::time_window::TimeWindow;
    use chrono::{TimeZone, Utc};

    fn snap(fetched_at: chrono::DateTime<Utc>, used: f64, limit: f64) -> UsageSnapshot {
        UsageSnapshot {
            provider: ProviderId::Claude,
            fetched_at,
            plan: None,
            buckets: vec![QuotaBucket {
                metric: "requests".into(),
                label: "Requests".into(),
                window: TimeWindow::session("5h", 5 * 3600),
                used: Some(Labeled::official(used)),
                limit: Some(Labeled::official(limit)),
                percent_remaining: None,
            }],
            source_strategy: "oauth_usage".into(),
            notes: vec![],
        }
    }

    fn ts(h: u32, m: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, h, m, 0).single().unwrap()
    }

    #[test]
    fn burn_rate_computed_from_increasing_used() {
        let snapshots = vec![snap(ts(10, 0), 10.0, 100.0), snap(ts(11, 0), 20.0, 100.0)];
        let metrics = compute_history_metrics(&snapshots);
        let br = metrics.burn_rate.unwrap();
        assert!((br.req_per_hour - 10.0).abs() < 0.01);
    }

    #[test]
    fn burn_rate_none_when_decreasing() {
        let snapshots = vec![snap(ts(10, 0), 20.0, 100.0), snap(ts(11, 0), 5.0, 100.0)];
        let metrics = compute_history_metrics(&snapshots);
        assert!(metrics.burn_rate.is_none());
    }

    #[test]
    fn weekly_peaks_groups_by_week() {
        let t1 = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).single().unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 8, 10, 0, 0).single().unwrap();
        let snapshots = vec![snap(t1, 50.0, 100.0), snap(t2, 80.0, 100.0)];
        let metrics = compute_history_metrics(&snapshots);
        assert_eq!(metrics.weekly_peaks.len(), 2);
        assert!((metrics.weekly_peaks[0] - 0.5).abs() < 0.01);
        assert!((metrics.weekly_peaks[1] - 0.8).abs() < 0.01);
    }
}
