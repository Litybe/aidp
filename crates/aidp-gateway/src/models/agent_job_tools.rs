use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Tool limits of a session. Names and ids only, never a server definition or a secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobTools {
    pub strict_mcp: bool,
    pub mcp_config_id: Option<String>,
    pub denied_mcp_tools: BTreeMap<String, Vec<String>>,
    pub denied_shell: Vec<String>,
    pub read_only: bool,
    pub no_shell: bool,
}
