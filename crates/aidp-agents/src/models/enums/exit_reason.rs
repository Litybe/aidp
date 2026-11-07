use serde::{Deserialize, Serialize};

/// Why an agent process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}
