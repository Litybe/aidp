use serde::{Deserialize, Serialize};
use std::fmt;

/// Why an autonomous run picked the answer it gave to a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoAnswerReason {
    Recommended,
    FirstOption,
    NoOption,
}

impl AutoAnswerReason {
    /// The wire value stored in `Assumption.reason`.
    pub const fn as_str(self) -> &'static str {
        match self {
            AutoAnswerReason::Recommended => "recommended",
            AutoAnswerReason::FirstOption => "first-option",
            AutoAnswerReason::NoOption => "no-option",
        }
    }
}

impl fmt::Display for AutoAnswerReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
