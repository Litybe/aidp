use crate::models::agent_job::AgentJob;
use crate::models::agent_session_report::AgentSessionReport;
use crate::models::enums::agent_job_state::AgentJobState;
use crate::models::enums::agent_verdict::AgentVerdict;
use serde::{Deserialize, Serialize};

/// A job as the board persists it in `agent-jobs/<seq>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentJobRecord {
    pub job: AgentJob,
    pub state: AgentJobState,
    #[serde(default)]
    pub claim_token: Option<String>,
    #[serde(default)]
    pub lease_expires_ms: Option<i64>,
    #[serde(default)]
    pub claims: u32,
    #[serde(default)]
    pub resumed: bool,
    pub created_ms: i64,
    #[serde(default)]
    pub pending_since_ms: i64,
    #[serde(default)]
    pub claimed_ms: Option<i64>,
    #[serde(default)]
    pub result_digest: Option<String>,
    #[serde(default)]
    pub report: Option<AgentSessionReport>,
    #[serde(default)]
    pub verdict: Option<AgentVerdict>,
    #[serde(default)]
    pub repair_job_id: Option<String>,
    #[serde(default)]
    pub consumed: bool,
}
