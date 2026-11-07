use serde::{Deserialize, Serialize};

/// Whether a job is the first session of a label or its repair.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentJobKind {
    #[default]
    Initial,
    Repair,
}
