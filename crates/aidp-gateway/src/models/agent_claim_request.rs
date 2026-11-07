use serde::{Deserialize, Serialize};

/// Body of a claim: who takes the job and from which worker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentClaimRequest {
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub host_id: Option<String>,
}
