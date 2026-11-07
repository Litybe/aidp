use serde::{Deserialize, Serialize};

/// Who launches the agent sessions of a native run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentExecutor {
    /// The gateway launches every session itself.
    #[default]
    Internal,
    /// A host launches every session from a job the gateway publishes.
    External,
}

impl AgentExecutor {
    /// True for the default mode, which a request leaves out of its JSON.
    pub fn is_internal(&self) -> bool {
        matches!(self, AgentExecutor::Internal)
    }
}
