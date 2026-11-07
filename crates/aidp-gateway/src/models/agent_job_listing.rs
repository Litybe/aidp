use crate::models::enums::agent_job_kind::AgentJobKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The `agent` object of a job in the flow's external listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobListing {
    pub run_id: String,
    pub stage: String,
    pub label: String,
    pub job_kind: AgentJobKind,
    pub resumed: bool,
    pub provider: String,
    pub tier: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub cwd: PathBuf,
}
