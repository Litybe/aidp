use serde::{Deserialize, Serialize};

/// State of one stage, plan task or shard of a native run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeStageStatus {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
    Cancelled,
}

impl NativeStageStatus {
    /// Wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
        }
    }

    /// True when the stage needs no further work in this run.
    pub const fn is_settled(self) -> bool {
        matches!(self, Self::Done | Self::Skipped)
    }
}
