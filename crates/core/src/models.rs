use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::confidence::Labeled;
use crate::time_window::TimeWindow;

/// Identifies which provider a piece of data belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Codex,
    Claude,
    Copilot,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderId::Codex => "codex",
            ProviderId::Claude => "claude",
            ProviderId::Copilot => "copilot",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderId::Codex => "Codex / ChatGPT",
            ProviderId::Claude => "Claude",
            ProviderId::Copilot => "GitHub Copilot",
        }
    }

    /// All supported providers.
    pub fn all() -> &'static [ProviderId] {
        &[ProviderId::Codex, ProviderId::Claude, ProviderId::Copilot]
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

impl std::str::FromStr for ProviderId {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "codex" | "chatgpt" | "openai" => Ok(ProviderId::Codex),
            "claude" | "anthropic" => Ok(ProviderId::Claude),
            "copilot" | "github" => Ok(ProviderId::Copilot),
            _ => Err(crate::error::CoreError::UnknownProvider(s.to_string())),
        }
    }
}

/// The plan/tier the user is on for a given provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanInfo {
    pub name: Labeled<String>,
    /// e.g. "Plus", "Pro", "Free"
    pub tier: Option<Labeled<String>>,
}

impl PlanInfo {
    fn same_confidence_display(&self) -> bool {
        self.tier
            .as_ref()
            .map(|tier| tier.confidence == self.name.confidence)
            .unwrap_or(false)
    }

    pub fn display_text(&self) -> String {
        match &self.tier {
            Some(tier) if self.same_confidence_display() => {
                format!(
                    "{} / {} [{}]",
                    self.name.value, tier.value, self.name.confidence
                )
            }
            Some(tier) => format!(
                "{} [{}] / {} [{}]",
                self.name.value, self.name.confidence, tier.value, tier.confidence
            ),
            None => format!("{} [{}]", self.name.value, self.name.confidence),
        }
    }
}

/// A single quota bucket (e.g. "session interactions", "weekly premium requests").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaBucket {
    /// What this bucket measures, e.g. "premium_requests", "interactions", "chat".
    pub metric: String,
    /// Human-readable label, e.g. "Premium Requests".
    pub label: String,
    /// The time window this bucket covers.
    pub window: TimeWindow,
    /// Amount used in this window.
    pub used: Option<Labeled<f64>>,
    /// Total quota limit in this window.
    pub limit: Option<Labeled<f64>>,
    /// Percentage remaining (0.0 - 1.0). Some providers give this directly.
    pub percent_remaining: Option<Labeled<f64>>,
}

impl QuotaBucket {
    /// Compute percent remaining, preferring the directly-provided value,
    /// falling back to used/limit calculation.
    pub fn effective_percent_remaining(&self) -> Option<Labeled<f64>> {
        if let Some(ref pr) = self.percent_remaining {
            return Some(pr.clone());
        }
        match (&self.used, &self.limit) {
            (Some(u), Some(l)) if l.value > 0.0 => {
                let remaining = ((l.value - u.value) / l.value).clamp(0.0, 1.0);
                // Derived because we computed it ourselves
                Some(Labeled::derived(remaining))
            }
            _ => None,
        }
    }
}

/// The full usage snapshot for one provider at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub provider: ProviderId,
    pub fetched_at: DateTime<Utc>,
    /// The plan/tier info if available.
    pub plan: Option<PlanInfo>,
    /// All quota buckets for this provider.
    pub buckets: Vec<QuotaBucket>,
    /// Which fetch strategy produced this snapshot.
    pub source_strategy: String,
    /// Any warnings/notes from the fetch (e.g. "rate limited, showing cached data").
    pub notes: Vec<String>,
}

impl UsageSnapshot {
    /// Find a bucket by metric name.
    pub fn bucket(&self, metric: &str) -> Option<&QuotaBucket> {
        self.buckets.iter().find(|b| b.metric == metric)
    }

    /// Get the most critical (lowest remaining) bucket.
    pub fn most_critical_bucket(&self) -> Option<&QuotaBucket> {
        self.buckets
            .iter()
            .filter_map(|b| b.effective_percent_remaining().map(|pr| (b, pr.value)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(b, _)| b)
    }

    /// Check if this snapshot is older than the given number of seconds.
    pub fn is_older_than(&self, secs: u64) -> bool {
        let age = Utc::now() - self.fetched_at;
        age.num_seconds() > secs as i64
    }

    /// Mark all labeled values in this snapshot as stale.
    /// Returns a new snapshot with all confidence levels set to Stale.
    pub fn mark_stale(mut self) -> Self {
        if let Some(ref mut plan) = self.plan {
            plan.name = plan.name.clone().mark_stale();
            plan.tier = plan.tier.clone().map(|t| t.mark_stale());
        }
        for bucket in &mut self.buckets {
            bucket.used = bucket.used.clone().map(|u| u.mark_stale());
            bucket.limit = bucket.limit.clone().map(|l| l.mark_stale());
            bucket.percent_remaining = bucket.percent_remaining.clone().map(|p| p.mark_stale());
        }
        if !self.notes.iter().any(|n| n.contains("stale")) {
            self.notes.push("Data is stale (older than TTL)".into());
        }
        self
    }
}

/// Authentication state for a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    /// Not configured at all.
    NotConfigured,
    /// Credentials stored but not yet validated.
    Configured,
    /// Successfully authenticated and token is valid.
    Authenticated,
    /// Token expired, needs refresh.
    Expired,
    /// Authentication failed (bad credentials, revoked, etc.).
    Failed(String),
}

impl AuthState {
    pub fn is_usable(&self) -> bool {
        matches!(self, AuthState::Authenticated | AuthState::Configured)
    }
}

impl std::fmt::Display for AuthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthState::NotConfigured => f.write_str("not configured"),
            AuthState::Configured => f.write_str("configured"),
            AuthState::Authenticated => f.write_str("authenticated"),
            AuthState::Expired => f.write_str("expired"),
            AuthState::Failed(msg) => write!(f, "failed: {msg}"),
        }
    }
}

/// Summary of a provider's current status (for CLI/TUI display).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub provider: ProviderId,
    pub auth_state: AuthState,
    pub last_snapshot: Option<UsageSnapshot>,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::PlanInfo;
    use crate::confidence::Labeled;

    #[test]
    fn display_text_formats_name_only() {
        let plan = PlanInfo {
            name: Labeled::provider_local("Claude".to_string()),
            tier: None,
        };

        assert_eq!(plan.display_text(), "Claude [local]");
    }

    #[test]
    fn display_text_formats_same_confidence_name_and_tier() {
        let plan = PlanInfo {
            name: Labeled::provider_local("Claude".to_string()),
            tier: Some(Labeled::provider_local("pro".to_string())),
        };

        assert_eq!(plan.display_text(), "Claude / pro [local]");
    }

    #[test]
    fn display_text_formats_different_confidences() {
        let plan = PlanInfo {
            name: Labeled::experimental("Claude".to_string()),
            tier: Some(Labeled::provider_local("pro".to_string())),
        };

        assert_eq!(plan.display_text(), "Claude [experimental] / pro [local]");
    }
}
