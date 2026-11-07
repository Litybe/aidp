use crate::models::agent_job_listing::AgentJobListing;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A pending job as the listing shows it: the base fields of an external job plus its `agent` object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobSummary {
    pub id: String,
    /// The row label: the stage, or `<stage>#<id>` for a task or shard.
    pub name: String,
    pub attempt: u32,
    pub writes: bool,
    pub workspace: PathBuf,
    pub created_at: String,
    pub agent: AgentJobListing,
}
