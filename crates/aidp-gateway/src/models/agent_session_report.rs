use crate::models::agent_usage_report::AgentUsageReport;
use serde::{Deserialize, Serialize};

/// The `agent` block of a result: the claim, the session and what it measured.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionReport {
    #[serde(default)]
    pub claim_token: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub resolved_model: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub last_message: Option<String>,
    #[serde(default)]
    pub reply_file: Option<String>,
    #[serde(default)]
    pub retry_after_seconds: Option<u64>,
    #[serde(default)]
    pub usage: Option<AgentUsageReport>,
}
