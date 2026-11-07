use serde::{Deserialize, Serialize};

/// What a review stage audits, read from its `inputs.axis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewAxis {
    Correctness,
    Security,
    Performance,
    Maintainability,
    Synthesis,
}

impl ReviewAxis {
    /// Parses an `inputs.axis` value, `None` when it names no review axis.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "correctness" => Some(Self::Correctness),
            "security" => Some(Self::Security),
            "performance" => Some(Self::Performance),
            "maintainability" => Some(Self::Maintainability),
            "synthesis" => Some(Self::Synthesis),
            _ => None,
        }
    }

    /// Whether this axis audits a diff itself rather than merging the other axes.
    pub const fn is_dimension(&self) -> bool {
        !matches!(self, Self::Synthesis)
    }
}
