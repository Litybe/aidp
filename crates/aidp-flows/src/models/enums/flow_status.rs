use serde::{Deserialize, Serialize};

/// Status of a whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowStatus {
    Running,
    Paused,
    Pausing,
    WaitingHuman,
    WaitingExternal,
    Done,
    Failed,
    Escalated,
    Aborted,
}

impl FlowStatus {
    /// True once the run stopped by itself or by decision.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            FlowStatus::Done | FlowStatus::Failed | FlowStatus::Escalated | FlowStatus::Aborted
        )
    }

    /// The snake_case wire value of this status.
    pub fn as_wire(self) -> &'static str {
        match self {
            FlowStatus::Running => "running",
            FlowStatus::Paused => "paused",
            FlowStatus::Pausing => "pausing",
            FlowStatus::WaitingHuman => "waiting_human",
            FlowStatus::WaitingExternal => "waiting_external",
            FlowStatus::Done => "done",
            FlowStatus::Failed => "failed",
            FlowStatus::Escalated => "escalated",
            FlowStatus::Aborted => "aborted",
        }
    }
}
