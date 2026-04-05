use serde::{Deserialize, Serialize};
use std::fmt;

/// The kinds of time windows providers use for quota/usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// A rolling N-hour session window (e.g. Codex 5-hour, Claude 5-hour).
    Session,
    /// A calendar/rolling weekly window (e.g. 7-day).
    Weekly,
    /// A calendar monthly window.
    Monthly,
    /// A daily window.
    Daily,
}

impl fmt::Display for WindowKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowKind::Session => f.write_str("session"),
            WindowKind::Weekly => f.write_str("weekly"),
            WindowKind::Monthly => f.write_str("monthly"),
            WindowKind::Daily => f.write_str("daily"),
        }
    }
}

/// Describes a specific time window with its boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindow {
    pub kind: WindowKind,
    /// Human-readable label, e.g. "5-hour session", "7-day rolling".
    pub label: String,
    /// Duration of the window in seconds.
    pub duration_secs: Option<u64>,
    /// When this window resets (if known).
    pub resets_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl TimeWindow {
    pub fn session(label: impl Into<String>, duration_secs: u64) -> Self {
        Self {
            kind: WindowKind::Session,
            label: label.into(),
            duration_secs: Some(duration_secs),
            resets_at: None,
        }
    }

    pub fn weekly(label: impl Into<String>) -> Self {
        Self {
            kind: WindowKind::Weekly,
            label: label.into(),
            duration_secs: Some(7 * 24 * 3600),
            resets_at: None,
        }
    }

    pub fn with_reset(mut self, resets_at: chrono::DateTime<chrono::Utc>) -> Self {
        self.resets_at = Some(resets_at);
        self
    }

    /// How much time remains until reset, if known.
    pub fn time_remaining(&self) -> Option<chrono::Duration> {
        self.resets_at
            .map(|r| r - chrono::Utc::now())
            .filter(|d| d.num_seconds() > 0)
    }
}
