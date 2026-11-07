use serde::{Deserialize, Serialize};

/// What a cancelled or failed fan-out item does to the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemFailurePolicy {
    #[default]
    Pause,
    Continue,
}

/// Whether read-only roles resolve a tool-restricted `agentType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEnforcement {
    #[default]
    Off,
    Strict,
}

/// What a stage does when its reply reports a harness gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessGapPolicy {
    Pause,
    #[default]
    Continue,
}

/// Where a stage's sub-agent runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    #[default]
    Worktree,
}
