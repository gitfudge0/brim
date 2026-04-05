use serde::{Deserialize, Serialize};
use std::fmt;

/// How much we trust a particular data point.
///
/// Every metric displayed to the user MUST carry one of these labels.
/// The UI must never present `Experimental` data as if it were `Official`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Data from an officially supported, documented API.
    Official,
    /// Data read from the provider's own local files/CLI (e.g. auth.json).
    ProviderLocal,
    /// Data obtained via undocumented/internal APIs that may break.
    Experimental,
    /// Data computed from other data points (e.g. inferring reset time).
    Derived,
    /// Data that was once fresh but is now past its TTL.
    Stale,
}

impl Confidence {
    /// Returns true if this confidence level should trigger a visual warning.
    pub fn needs_warning(self) -> bool {
        matches!(self, Confidence::Experimental | Confidence::Stale)
    }

    /// Human-readable short label for display.
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Official => "official",
            Confidence::ProviderLocal => "local",
            Confidence::Experimental => "experimental",
            Confidence::Derived => "derived",
            Confidence::Stale => "stale",
        }
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A value annotated with its confidence/source label.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Labeled<T> {
    pub value: T,
    pub confidence: Confidence,
}

impl<T> Labeled<T> {
    pub fn new(value: T, confidence: Confidence) -> Self {
        Self { value, confidence }
    }

    pub fn official(value: T) -> Self {
        Self::new(value, Confidence::Official)
    }

    pub fn experimental(value: T) -> Self {
        Self::new(value, Confidence::Experimental)
    }

    pub fn derived(value: T) -> Self {
        Self::new(value, Confidence::Derived)
    }

    pub fn provider_local(value: T) -> Self {
        Self::new(value, Confidence::ProviderLocal)
    }

    /// Mark this labeled value as stale (preserving the inner value).
    pub fn mark_stale(self) -> Self {
        Self {
            value: self.value,
            confidence: Confidence::Stale,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Labeled<U> {
        Labeled {
            value: f(self.value),
            confidence: self.confidence,
        }
    }
}
