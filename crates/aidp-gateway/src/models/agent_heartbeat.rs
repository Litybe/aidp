use serde::{Deserialize, Serialize};

/// Answer to a lease renewal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHeartbeat {
    pub lease_expires_at: String,
    pub cancel_requested: bool,
}
