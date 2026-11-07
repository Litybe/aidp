use serde::{Deserialize, Serialize};

/// Lifecycle state of a native run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRunStatus {
    Pending,
    Running,
    /// Stopped before a gated stage until a human approves it.
    WaitingGate,
    /// Stopped on a stage that needs a human: retry, skip or cancel.
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl NativeRunStatus {
    /// Wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::WaitingGate => "waiting_gate",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// True once the run can no longer change without a new run.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}
