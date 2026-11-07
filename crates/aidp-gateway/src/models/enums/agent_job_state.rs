use serde::{Deserialize, Serialize};

/// Where an agent job is in its life.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentJobState {
    #[default]
    Pending,
    Claimed,
    Done,
    Cancelled,
}

impl AgentJobState {
    /// True while the job still waits for a host or runs on one.
    pub fn is_open(&self) -> bool {
        matches!(self, AgentJobState::Pending | AgentJobState::Claimed)
    }
}
