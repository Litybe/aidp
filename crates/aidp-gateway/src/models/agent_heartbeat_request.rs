use serde::{Deserialize, Serialize};

/// Body of a lease renewal.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHeartbeatRequest {
    #[serde(default)]
    pub claim_token: String,
}
