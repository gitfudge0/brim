use std::cmp::Ordering;
use std::collections::BTreeMap;

use anyhow::Result;
use brim_core::confidence::{Confidence, Labeled};
use brim_core::history::{compute_history_metrics, HistoryMetrics};
use brim_core::models::{AuthState, ProviderStatus, QuotaBucket, UsageSnapshot};
use brim_core::time_window::WindowKind;
use brim_providers::sync_engine::SyncEngine;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct JsonProviderSummary {
    provider: String,
    enabled: bool,
    auth: &'static str,
    plan: Option<String>,
    tier: Option<String>,
    fetched_at: Option<DateTime<Utc>>,
    stale: bool,
    source: Option<String>,
    notes: Vec<String>,
    lowest_bucket: Option<JsonLowestBucketSummary>,
    buckets: Vec<JsonBucketSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history: Option<HistoryMetrics>,
}

#[derive(Debug, Serialize)]
struct JsonBucketSummary {
    metric: String,
    label: String,
    window: String,
    used: Option<f64>,
    limit: Option<f64>,
    remaining_pct: Option<f64>,
    resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct JsonLowestBucketSummary {
    metric: String,
    label: String,
    window: String,
    remaining_pct: Option<f64>,
    resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct CompactJsonOutput {
    version: &'static str,
    usage: BTreeMap<String, CompactProviderUsage>,
}

#[derive(Debug, Default, Serialize)]
struct CompactProviderUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<CompactWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    weekly: Option<CompactWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    monthly: Option<CompactWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    daily: Option<CompactWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history: Option<HistoryMetrics>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
struct CompactWindowSummary {
    remaining_pct: Option<f64>,
    resets_at: Option<DateTime<Utc>>,
}

pub async fn run(
    engine: &SyncEngine,
    provider: Option<String>,
    fresh: bool,
    full: bool,
    history: bool,
) -> Result<()> {
    let statuses =
        crate::commands::status::collect_statuses(engine, provider.clone(), fresh).await?;

    // Build per-provider history metrics if requested.
    let history_map: BTreeMap<String, HistoryMetrics> = if history {
        let ids: Vec<brim_core::models::ProviderId> = match &provider {
            Some(name) => vec![crate::commands::parse_provider_arg(name)?],
            None => engine.config().enabled_provider_ids(),
        };
        ids.into_iter()
            .filter_map(|id| {
                engine
                    .db()
                    .snapshots_for_history(id, 60)
                    .ok()
                    .map(|snaps| (id.as_str().to_string(), compute_history_metrics(&snaps)))
            })
            .collect()
    } else {
        BTreeMap::new()
    };

    let json = if full {
        let mut summaries = summarize_statuses_full(statuses);
        if history {
            for s in &mut summaries {
                s.history = history_map.get(&s.provider).cloned();
            }
        }
        serde_json::to_string_pretty(&summaries)?
    } else {
        let mut compact = summarize_statuses_compact(statuses);
        if history {
            for (key, usage) in &mut compact.usage {
                usage.history = history_map.get(key).cloned();
            }
        }
        serde_json::to_string_pretty(&compact)?
    };
    println!("{json}");
    Ok(())
}

fn summarize_statuses_full(statuses: Vec<ProviderStatus>) -> Vec<JsonProviderSummary> {
    statuses
        .into_iter()
        .map(JsonProviderSummary::from)
        .collect()
}

fn summarize_statuses_compact(statuses: Vec<ProviderStatus>) -> CompactJsonOutput {
    let mut usage = BTreeMap::new();

    for status in statuses {
        if let Some(snapshot) = status.last_snapshot {
            let provider_usage = CompactProviderUsage::from_snapshot(&snapshot);
            if !provider_usage.is_empty() {
                usage.insert(status.provider.as_str().to_string(), provider_usage);
            }
        }
    }

    CompactJsonOutput {
        version: env!("CARGO_PKG_VERSION"),
        usage,
    }
}

impl From<ProviderStatus> for JsonProviderSummary {
    fn from(status: ProviderStatus) -> Self {
        let provider = status.provider.as_str().to_string();
        let enabled = status.enabled;
        let auth = auth_label(&status.auth_state);

        match status.last_snapshot {
            Some(snapshot) => {
                let stale = snapshot_is_stale(&snapshot);
                let lowest_bucket = snapshot
                    .most_critical_bucket()
                    .map(JsonLowestBucketSummary::from_bucket);
                let buckets = snapshot
                    .buckets
                    .iter()
                    .map(JsonBucketSummary::from_bucket)
                    .collect();

                Self {
                    provider,
                    enabled,
                    auth,
                    plan: snapshot.plan.as_ref().map(|plan| plan.name.value.clone()),
                    tier: snapshot
                        .plan
                        .as_ref()
                        .and_then(|plan| plan.tier.as_ref().map(|tier| tier.value.clone())),
                    fetched_at: Some(snapshot.fetched_at),
                    stale,
                    source: Some(snapshot.source_strategy.clone()),
                    notes: snapshot.notes.clone(),
                    lowest_bucket,
                    buckets,
                    history: None,
                }
            }
            None => Self {
                provider,
                enabled,
                auth,
                plan: None,
                tier: None,
                fetched_at: None,
                stale: false,
                source: None,
                notes: Vec::new(),
                lowest_bucket: None,
                buckets: Vec::new(),
                history: None,
            },
        }
    }
}

impl JsonBucketSummary {
    fn from_bucket(bucket: &QuotaBucket) -> Self {
        Self {
            metric: bucket.metric.clone(),
            label: bucket.label.clone(),
            window: canonical_window_name(bucket.window.kind).to_string(),
            used: bucket.used.as_ref().map(|value| value.value),
            limit: bucket.limit.as_ref().map(|value| value.value),
            remaining_pct: bucket
                .effective_percent_remaining()
                .map(|value| value.value),
            resets_at: bucket.window.resets_at,
        }
    }
}

impl JsonLowestBucketSummary {
    fn from_bucket(bucket: &QuotaBucket) -> Self {
        Self {
            metric: bucket.metric.clone(),
            label: bucket.label.clone(),
            window: canonical_window_name(bucket.window.kind).to_string(),
            remaining_pct: bucket
                .effective_percent_remaining()
                .map(|value| value.value),
            resets_at: bucket.window.resets_at,
        }
    }
}

impl CompactProviderUsage {
    fn from_snapshot(snapshot: &UsageSnapshot) -> Self {
        let mut usage = Self::default();
        let mut session: Option<&QuotaBucket> = None;
        let mut weekly: Option<&QuotaBucket> = None;
        let mut monthly: Option<&QuotaBucket> = None;
        let mut daily: Option<&QuotaBucket> = None;

        for bucket in &snapshot.buckets {
            match bucket.window.kind {
                WindowKind::Session => choose_better_bucket(&mut session, bucket),
                WindowKind::Weekly => choose_better_bucket(&mut weekly, bucket),
                WindowKind::Monthly => choose_better_bucket(&mut monthly, bucket),
                WindowKind::Daily => choose_better_bucket(&mut daily, bucket),
            }
        }

        usage.session = session.map(CompactWindowSummary::from_bucket);
        usage.weekly = weekly.map(CompactWindowSummary::from_bucket);
        usage.monthly = monthly.map(CompactWindowSummary::from_bucket);
        usage.daily = daily.map(CompactWindowSummary::from_bucket);
        usage
    }

    fn is_empty(&self) -> bool {
        self.session.is_none()
            && self.weekly.is_none()
            && self.monthly.is_none()
            && self.daily.is_none()
    }
}

impl CompactWindowSummary {
    fn from_bucket(bucket: &QuotaBucket) -> Self {
        Self {
            remaining_pct: bucket
                .effective_percent_remaining()
                .map(|value| value.value),
            resets_at: bucket.window.resets_at,
        }
    }
}

fn choose_better_bucket<'a>(slot: &mut Option<&'a QuotaBucket>, candidate: &'a QuotaBucket) {
    match slot {
        Some(current) if compare_bucket_priority(candidate, current) == Ordering::Less => {
            *slot = Some(candidate);
        }
        None => *slot = Some(candidate),
        _ => {}
    }
}

fn compare_bucket_priority(left: &QuotaBucket, right: &QuotaBucket) -> Ordering {
    match compare_remaining_pct(left, right) {
        Ordering::Equal => match compare_reset_presence(left, right) {
            Ordering::Equal => Ordering::Equal,
            other => other,
        },
        other => other,
    }
}

fn compare_remaining_pct(left: &QuotaBucket, right: &QuotaBucket) -> Ordering {
    match (
        left.effective_percent_remaining().map(|value| value.value),
        right.effective_percent_remaining().map(|value| value.value),
    ) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_reset_presence(left: &QuotaBucket, right: &QuotaBucket) -> Ordering {
    match (
        left.window.resets_at.is_some(),
        right.window.resets_at.is_some(),
    ) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

fn canonical_window_name(kind: WindowKind) -> &'static str {
    match kind {
        WindowKind::Session => "session",
        WindowKind::Weekly => "weekly",
        WindowKind::Monthly => "monthly",
        WindowKind::Daily => "daily",
    }
}

fn auth_label(auth_state: &AuthState) -> &'static str {
    match auth_state {
        AuthState::NotConfigured => "not_configured",
        AuthState::Configured => "configured",
        AuthState::Authenticated => "authenticated",
        AuthState::Expired => "expired",
        AuthState::Failed(_) => "failed",
    }
}

fn snapshot_is_stale(snapshot: &UsageSnapshot) -> bool {
    snapshot
        .notes
        .iter()
        .any(|note| note.to_lowercase().contains("stale"))
        || snapshot.plan.as_ref().is_some_and(|plan| {
            labeled_is_stale(&plan.name) || plan.tier.as_ref().is_some_and(labeled_is_stale)
        })
        || snapshot.buckets.iter().any(bucket_is_stale)
}

fn bucket_is_stale(bucket: &QuotaBucket) -> bool {
    bucket.used.as_ref().is_some_and(labeled_is_stale)
        || bucket.limit.as_ref().is_some_and(labeled_is_stale)
        || bucket
            .percent_remaining
            .as_ref()
            .is_some_and(labeled_is_stale)
}

fn labeled_is_stale<T>(value: &Labeled<T>) -> bool {
    value.confidence == Confidence::Stale
}

#[cfg(test)]
mod tests {
    use super::{summarize_statuses_compact, summarize_statuses_full};
    use brim_core::confidence::{Confidence, Labeled};
    use brim_core::models::{
        AuthState, PlanInfo, ProviderId, ProviderStatus, QuotaBucket, UsageSnapshot,
    };
    use brim_core::time_window::{TimeWindow, WindowKind};
    use chrono::{TimeZone, Utc};
    use serde_json::Value;

    #[test]
    fn full_mode_serializes_flattened_full_snapshot() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Codex,
            auth_state: AuthState::Configured,
            last_snapshot: Some(sample_snapshot()),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_full(statuses));

        assert_eq!(value[0]["provider"], "codex");
        assert_eq!(value[0]["enabled"], true);
        assert_eq!(value[0]["auth"], "configured");
        assert_eq!(value[0]["plan"], "plus");
        assert_eq!(value[0]["tier"], "pro");
        assert_eq!(value[0]["fetched_at"], "2026-03-31T17:54:15Z");
        assert_eq!(value[0]["stale"], false);
        assert_eq!(value[0]["source"], "cli_rpc");
        assert_eq!(value[0]["lowest_bucket"]["metric"], "secondary");
        assert_eq!(value[0]["lowest_bucket"]["window"], "weekly");
        assert_eq!(value[0]["lowest_bucket"]["remaining_pct"], 0.25);
        assert_eq!(value[0]["buckets"][0]["remaining_pct"], 0.8);
        assert_eq!(value[0]["buckets"][1]["remaining_pct"], 0.25);
        assert!(value[0].get("last_snapshot").is_none());
    }

    #[test]
    fn full_mode_emits_nulls_and_empty_arrays_without_snapshot() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Claude,
            auth_state: AuthState::NotConfigured,
            last_snapshot: None,
            enabled: false,
        }];

        let value = serialize(summarize_statuses_full(statuses));

        assert_eq!(value[0]["provider"], "claude");
        assert_eq!(value[0]["auth"], "not_configured");
        assert_eq!(value[0]["plan"], Value::Null);
        assert_eq!(value[0]["tier"], Value::Null);
        assert_eq!(value[0]["fetched_at"], Value::Null);
        assert_eq!(value[0]["source"], Value::Null);
        assert_eq!(value[0]["lowest_bucket"], Value::Null);
        assert_eq!(value[0]["notes"], Value::Array(Vec::new()));
        assert_eq!(value[0]["buckets"], Value::Array(Vec::new()));
    }

    #[test]
    fn full_mode_derives_remaining_percentage_from_used_and_limit() {
        let bucket = QuotaBucket {
            metric: "derived".into(),
            label: "Derived".into(),
            window: TimeWindow {
                kind: WindowKind::Session,
                label: "5-hour window".into(),
                duration_secs: Some(18_000),
                resets_at: None,
            },
            used: Some(Labeled::official(30.0)),
            limit: Some(Labeled::official(120.0)),
            percent_remaining: None,
        };

        let statuses = vec![ProviderStatus {
            provider: ProviderId::Copilot,
            auth_state: AuthState::Authenticated,
            last_snapshot: Some(UsageSnapshot {
                provider: ProviderId::Copilot,
                fetched_at: ts(2026, 4, 1, 0, 0, 0),
                plan: None,
                buckets: vec![bucket],
                source_strategy: "copilot_internal_user".into(),
                notes: Vec::new(),
            }),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_full(statuses));

        assert_eq!(value[0]["buckets"][0]["remaining_pct"], 0.75);
        assert_eq!(value[0]["lowest_bucket"]["remaining_pct"], 0.75);
    }

    #[test]
    fn full_mode_marks_summary_stale_when_snapshot_values_are_stale() {
        let mut snapshot = sample_snapshot();
        snapshot.plan.as_mut().unwrap().name.confidence = Confidence::Stale;

        let statuses = vec![ProviderStatus {
            provider: ProviderId::Codex,
            auth_state: AuthState::Configured,
            last_snapshot: Some(snapshot),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_full(statuses));

        assert_eq!(value[0]["stale"], true);
    }

    #[test]
    fn full_mode_preserves_array_shape_for_single_provider() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Codex,
            auth_state: AuthState::Configured,
            last_snapshot: Some(sample_snapshot()),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_full(statuses));

        assert!(value.is_array());
        assert_eq!(value.as_array().unwrap().len(), 1);
    }

    #[test]
    fn full_mode_keeps_failed_auth_with_empty_buckets_without_snapshot() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Copilot,
            auth_state: AuthState::Failed("token revoked".into()),
            last_snapshot: None,
            enabled: true,
        }];

        let value = serialize(summarize_statuses_full(statuses));

        assert_eq!(value[0]["auth"], "failed");
        assert_eq!(value[0]["buckets"], Value::Array(Vec::new()));
        assert_eq!(value[0]["lowest_bucket"], Value::Null);
    }

    #[test]
    fn compact_mode_uses_version_and_provider_usage_map() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Codex,
            auth_state: AuthState::Configured,
            last_snapshot: Some(sample_snapshot()),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_compact(statuses));

        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["usage"]["codex"]["session"]["remaining_pct"], 0.8);
        assert_eq!(value["usage"]["codex"]["weekly"]["remaining_pct"], 0.25);
        assert!(value["usage"].get("claude").is_none());
    }

    #[test]
    fn compact_mode_omits_missing_windows_and_full_fields() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Copilot,
            auth_state: AuthState::Configured,
            last_snapshot: Some(UsageSnapshot {
                provider: ProviderId::Copilot,
                fetched_at: ts(2026, 4, 1, 0, 0, 0),
                plan: None,
                buckets: vec![QuotaBucket {
                    metric: "premium_interactions".into(),
                    label: "Premium Requests".into(),
                    window: TimeWindow {
                        kind: WindowKind::Monthly,
                        label: "Monthly cycle".into(),
                        duration_secs: None,
                        resets_at: Some(ts(2026, 4, 1, 0, 0, 0)),
                    },
                    used: Some(Labeled::official(50.0)),
                    limit: Some(Labeled::official(100.0)),
                    percent_remaining: Some(Labeled::official(0.5)),
                }],
                source_strategy: "copilot_internal_user".into(),
                notes: vec!["experimental".into()],
            }),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_compact(statuses));

        assert_eq!(value["usage"]["copilot"]["monthly"]["remaining_pct"], 0.5);
        assert!(value["usage"]["copilot"].get("weekly").is_none());
        assert!(value["usage"]["copilot"].get("auth").is_none());
        assert!(value["usage"]["copilot"].get("notes").is_none());
        assert!(value["usage"]["copilot"].get("buckets").is_none());
    }

    #[test]
    fn compact_mode_preserves_reset_time_and_null_when_missing() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Codex,
            auth_state: AuthState::Configured,
            last_snapshot: Some(UsageSnapshot {
                provider: ProviderId::Codex,
                fetched_at: ts(2026, 3, 31, 17, 54, 15),
                plan: None,
                buckets: vec![
                    QuotaBucket {
                        metric: "primary".into(),
                        label: "Primary".into(),
                        window: TimeWindow {
                            kind: WindowKind::Session,
                            label: "5-hour window".into(),
                            duration_secs: Some(18_000),
                            resets_at: Some(ts(2026, 3, 31, 22, 54, 14)),
                        },
                        used: None,
                        limit: None,
                        percent_remaining: Some(Labeled::official(0.8)),
                    },
                    QuotaBucket {
                        metric: "secondary".into(),
                        label: "Secondary".into(),
                        window: TimeWindow {
                            kind: WindowKind::Weekly,
                            label: "7-day window".into(),
                            duration_secs: Some(604_800),
                            resets_at: None,
                        },
                        used: None,
                        limit: None,
                        percent_remaining: Some(Labeled::official(0.25)),
                    },
                ],
                source_strategy: "cli_rpc".into(),
                notes: Vec::new(),
            }),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_compact(statuses));

        assert_eq!(
            value["usage"]["codex"]["session"]["resets_at"],
            "2026-03-31T22:54:14Z"
        );
        assert_eq!(value["usage"]["codex"]["weekly"]["resets_at"], Value::Null);
    }

    #[test]
    fn compact_mode_chooses_lowest_remaining_bucket_per_window_kind() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Claude,
            auth_state: AuthState::Configured,
            last_snapshot: Some(UsageSnapshot {
                provider: ProviderId::Claude,
                fetched_at: ts(2026, 4, 1, 0, 0, 0),
                plan: None,
                buckets: vec![
                    QuotaBucket {
                        metric: "model_a".into(),
                        label: "Model A".into(),
                        window: TimeWindow {
                            kind: WindowKind::Session,
                            label: "5-hour window".into(),
                            duration_secs: Some(18_000),
                            resets_at: Some(ts(2026, 4, 1, 5, 0, 0)),
                        },
                        used: None,
                        limit: None,
                        percent_remaining: Some(Labeled::official(0.8)),
                    },
                    QuotaBucket {
                        metric: "model_b".into(),
                        label: "Model B".into(),
                        window: TimeWindow {
                            kind: WindowKind::Session,
                            label: "5-hour window".into(),
                            duration_secs: Some(18_000),
                            resets_at: Some(ts(2026, 4, 1, 6, 0, 0)),
                        },
                        used: None,
                        limit: None,
                        percent_remaining: Some(Labeled::official(0.3)),
                    },
                ],
                source_strategy: "oauth_usage".into(),
                notes: Vec::new(),
            }),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_compact(statuses));

        assert_eq!(value["usage"]["claude"]["session"]["remaining_pct"], 0.3);
        assert_eq!(
            value["usage"]["claude"]["session"]["resets_at"],
            "2026-04-01T06:00:00Z"
        );
    }

    #[test]
    fn compact_mode_prefers_bucket_with_remaining_pct_over_unknown() {
        let statuses = vec![ProviderStatus {
            provider: ProviderId::Claude,
            auth_state: AuthState::Configured,
            last_snapshot: Some(UsageSnapshot {
                provider: ProviderId::Claude,
                fetched_at: ts(2026, 4, 1, 0, 0, 0),
                plan: None,
                buckets: vec![
                    QuotaBucket {
                        metric: "unknown".into(),
                        label: "Unknown".into(),
                        window: TimeWindow {
                            kind: WindowKind::Session,
                            label: "5-hour window".into(),
                            duration_secs: Some(18_000),
                            resets_at: None,
                        },
                        used: None,
                        limit: None,
                        percent_remaining: None,
                    },
                    QuotaBucket {
                        metric: "known".into(),
                        label: "Known".into(),
                        window: TimeWindow {
                            kind: WindowKind::Session,
                            label: "5-hour window".into(),
                            duration_secs: Some(18_000),
                            resets_at: None,
                        },
                        used: None,
                        limit: None,
                        percent_remaining: Some(Labeled::official(0.4)),
                    },
                ],
                source_strategy: "oauth_usage".into(),
                notes: Vec::new(),
            }),
            enabled: true,
        }];

        let value = serialize(summarize_statuses_compact(statuses));

        assert_eq!(value["usage"]["claude"]["session"]["remaining_pct"], 0.4);
    }

    fn serialize<T: serde::Serialize>(value: T) -> Value {
        serde_json::to_value(value).unwrap()
    }

    fn sample_snapshot() -> UsageSnapshot {
        UsageSnapshot {
            provider: ProviderId::Codex,
            fetched_at: ts(2026, 3, 31, 17, 54, 15),
            plan: Some(PlanInfo {
                name: Labeled::provider_local("plus".into()),
                tier: Some(Labeled::provider_local("pro".into())),
            }),
            buckets: vec![
                QuotaBucket {
                    metric: "primary".into(),
                    label: "Primary (session)".into(),
                    window: TimeWindow {
                        kind: WindowKind::Session,
                        label: "5-hour window".into(),
                        duration_secs: Some(18_000),
                        resets_at: Some(ts(2026, 3, 31, 22, 54, 14)),
                    },
                    used: Some(Labeled::official(20.0)),
                    limit: Some(Labeled::official(100.0)),
                    percent_remaining: Some(Labeled::official(0.8)),
                },
                QuotaBucket {
                    metric: "secondary".into(),
                    label: "Secondary (weekly)".into(),
                    window: TimeWindow {
                        kind: WindowKind::Weekly,
                        label: "7-day window".into(),
                        duration_secs: Some(604_800),
                        resets_at: Some(ts(2026, 4, 4, 4, 10, 50)),
                    },
                    used: Some(Labeled::official(75.0)),
                    limit: Some(Labeled::official(100.0)),
                    percent_remaining: Some(Labeled::official(0.25)),
                },
            ],
            source_strategy: "cli_rpc".into(),
            notes: vec!["Data from Codex app-server JSON-RPC (provider-local)".into()],
        }
    }

    fn ts(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .unwrap()
    }
}
