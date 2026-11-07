use serde::{Deserialize, Serialize};

/// Status of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    #[default]
    Pending,
    Running,
    WaitingHuman,
    WaitingExternal,
    Done,
    Skipped,
    Failed,
    Escalated,
}

impl StepStatus {
    /// The snake_case wire value of this status.
    pub fn as_wire(self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::WaitingHuman => "waiting_human",
            StepStatus::WaitingExternal => "waiting_external",
            StepStatus::Done => "done",
            StepStatus::Skipped => "skipped",
            StepStatus::Failed => "failed",
            StepStatus::Escalated => "escalated",
        }
    }
}
