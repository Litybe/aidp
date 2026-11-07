use crate::models::agent_session_report::AgentSessionReport;
use serde::{Deserialize, Serialize};

/// The result a host posts for an agent job. `outcome` stays text so an unknown one is refused with a precise message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentJobResult {
    #[serde(default)]
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentSessionReport>,
}
