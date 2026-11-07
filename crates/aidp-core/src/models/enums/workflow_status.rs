use crate::models::constants::workflow_result::STATUS_KEY;
use serde::{Deserialize, Serialize};

/// Status a Workflow result object reports for one pass of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Done,
    Paused,
    AwaitingGate,
    AwaitingPlan,
    AwaitingComposite,
    #[serde(other)]
    Unknown,
}

impl WorkflowStatus {
    /// Parses a wire value, mapping anything unrecognised to `Unknown`.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "done" => Self::Done,
            "paused" => Self::Paused,
            "awaiting_gate" => Self::AwaitingGate,
            "awaiting_plan" => Self::AwaitingPlan,
            "awaiting_composite" => Self::AwaitingComposite,
            _ => Self::Unknown,
        }
    }

    /// Wire value, identical to the serialized form.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Paused => "paused",
            Self::AwaitingGate => "awaiting_gate",
            Self::AwaitingPlan => "awaiting_plan",
            Self::AwaitingComposite => "awaiting_composite",
            Self::Unknown => "unknown",
        }
    }

    /// Reads the status of a Workflow result object, `None` when it carries none.
    pub fn from_result(result: &serde_json::Value) -> Option<Self> {
        result
            .get(STATUS_KEY)
            .and_then(|v| v.as_str())
            .map(Self::parse)
    }
}
