use crate::models::agent_job::AgentJob;
use serde::{Deserialize, Serialize};

/// A claimed job with the token and lease that protect it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentClaim {
    pub claim_token: String,
    pub lease_seconds: u64,
    pub lease_expires_at: String,
    pub resumed: bool,
    pub job: AgentJob,
}
