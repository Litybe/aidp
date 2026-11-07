use serde::{Deserialize, Serialize};

/// Status a sub-agent reply envelope reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeStatus {
    Completed,
    NeedsReview,
    Blocked,
    Failed,
    #[default]
    #[serde(other)]
    Unknown,
}

impl EnvelopeStatus {
    /// Parses a wire value, mapping anything unrecognised to `Unknown`.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "completed" => Self::Completed,
            "needs_review" => Self::NeedsReview,
            "blocked" => Self::Blocked,
            "failed" => Self::Failed,
            _ => Self::Unknown,
        }
    }

    /// Wire value, identical to the serialized form.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::NeedsReview => "needs_review",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }

    /// True for a reply whose stage counts as having succeeded.
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Completed | Self::NeedsReview)
    }
}
