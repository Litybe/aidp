use serde::{Deserialize, Serialize};

/// Answer to a posted result: whether it counted and what happens next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobAck {
    pub job_id: String,
    pub accepted: bool,
    pub duplicate: bool,
    pub next: String,
    pub repair_job_id: Option<String>,
    pub reason: Option<String>,
}
