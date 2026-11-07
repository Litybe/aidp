use serde::{Deserialize, Serialize};

/// Who runs the actions and wait probes of a flow execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowExecutorMode {
    /// The gateway spawns the `aidp` subprocess of every action and polls every probe.
    #[default]
    Internal,
    /// The caller runs every action and probe and posts the results.
    External,
}

impl FlowExecutorMode {
    /// True for the default mode, which a policy leaves out of its JSON.
    pub fn is_internal(&self) -> bool {
        matches!(self, FlowExecutorMode::Internal)
    }
}
