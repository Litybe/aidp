use serde::{Deserialize, Serialize};

/// Who runs the session: role, provider, model tier, model pin and effort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobAgent {
    pub role: String,
    pub agent_type: String,
    pub provider: String,
    pub tier: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub ultracode: bool,
}
