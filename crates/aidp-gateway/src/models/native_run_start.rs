use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::models::enums::agent_executor::AgentExecutor;
use crate::sessions::Origin;

/// What it takes to start a native run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartNativeRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_mcp_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied_shell_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub denied_mcp_tools: BTreeMap<String, Vec<String>>,
    pub project_id: String,
    pub template_id: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub target: Option<String>,
    /// Directory the stage sessions run in; defaults to the project checkout.
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// Provider id overriding the config default for every stage without its own.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub provider_by_role: BTreeMap<String, String>,
    #[serde(default)]
    pub autonomous: bool,
    /// Every stage session is headless, see [`crate::sessions::StartRequest::headless`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub headless: bool,
    /// Stages whose gate is crossed without asking; `*` means every gate.
    #[serde(default)]
    pub auto_approve_gates: Vec<String>,
    /// Where the launch comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// Model overriding the stage tier for every stage session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Permission mode of the stage sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Reasoning effort of the stage sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Ticket key the run is tracked under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    /// Vault root that holds the run directory; unset means the vault the daemon resolves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<PathBuf>,
    /// Keep the run directory in the workspace (`.aidp/pipelines/`) and ignore every vault.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_vault: bool,
    /// Who launches the agent sessions: the gateway, or a host that claims one job per session.
    #[serde(default, skip_serializing_if = "AgentExecutor::is_internal")]
    pub agent_executor: AgentExecutor,
}
