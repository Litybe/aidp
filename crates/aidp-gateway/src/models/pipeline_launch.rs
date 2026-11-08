use crate::models::enums::agent_executor::AgentExecutor;
use crate::models::enums::engine::Engine;
use crate::models::ProjectRef;
use std::path::PathBuf;

/// One `pipeline` step to run as a session.
#[derive(Debug, Clone)]
pub struct PipelineLaunch {
    pub key: String,
    pub ticket_key: String,
    pub provider: Option<String>,
    pub permission_mode: Option<String>,
    pub strict_mcp_config: Option<bool>,
    pub mcp_config_id: Option<String>,
    pub mcp_servers: Option<Vec<serde_json::Value>>,
    pub denied_shell_commands: Vec<String>,
    pub denied_mcp_tools: std::collections::BTreeMap<String, Vec<String>>,
    pub step: String,
    pub template: String,
    pub project: ProjectRef,
    pub inputs_file: PathBuf,
    pub auto_approve_gates: Option<Vec<String>>,
    pub autonomous: bool,
    pub headless: bool,
    pub engine: Option<Engine>,
    /// Who launches the agent sessions of the run.
    pub agent_executor: AgentExecutor,
    /// True when the run works in the project's main checkout (flow `workspace: project`): no worktree.
    pub project_checkout: bool,
}
