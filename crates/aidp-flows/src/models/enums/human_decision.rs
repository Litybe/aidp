use serde::{Deserialize, Serialize};

/// A decision a human can take on a waiting step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanDecision {
    Approve,
    Reject,
    Click,
    Ok,
    Ko,
}

impl HumanDecision {
    /// The wire value of the decision.
    pub fn as_str(self) -> &'static str {
        match self {
            HumanDecision::Approve => "approve",
            HumanDecision::Reject => "reject",
            HumanDecision::Click => "click",
            HumanDecision::Ok => "ok",
            HumanDecision::Ko => "ko",
        }
    }

    /// Parses a wire value; `None` for a custom choice.
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Approve, Self::Reject, Self::Click, Self::Ok, Self::Ko]
            .into_iter()
            .find(|decision| decision.as_str() == value)
    }

    /// True for the decisions that count as a positive outcome.
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Approve | Self::Click | Self::Ok)
    }
}
