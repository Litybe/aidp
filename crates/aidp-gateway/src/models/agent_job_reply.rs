use serde::{Deserialize, Serialize};

/// The reply contract of a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobReply {
    pub schema: String,
    pub location: String,
    pub reviewer_signals: bool,
}
