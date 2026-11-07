use serde::{Deserialize, Serialize};

/// One-word verdict for a whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Failed,
    Done,
    InProgress,
    AwaitingGate,
    Pending,
    Paused,
}

impl RunStatus {
    /// Wire value, identical to the serialized form.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Done => "done",
            Self::InProgress => "in_progress",
            Self::AwaitingGate => "awaiting_gate",
            Self::Pending => "pending",
            Self::Paused => "paused",
        }
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<RunStatus> for String {
    fn from(status: RunStatus) -> Self {
        status.as_str().to_string()
    }
}
