use crate::models::enums::agent_executor::AgentExecutor;
use crate::models::native_run_start::StartNativeRun;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Body of `POST /api/runs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRunRequest {
    #[serde(default)]
    pub client_request_id: Option<String>,
    #[serde(default)]
    pub strict_mcp_config: Option<bool>,
    #[serde(default)]
    pub mcp_config_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<Value>>,
    #[serde(default)]
    pub denied_shell_commands: Vec<String>,
    #[serde(default)]
    pub denied_mcp_tools: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub provider_by_role: BTreeMap<String, String>,
    #[serde(default)]
    pub autonomous: bool,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub auto_approve_gates: Vec<String>,
    #[serde(default)]
    pub vault: Option<PathBuf>,
    #[serde(default)]
    pub no_vault: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl From<NativeRunRequest> for StartNativeRun {
    fn from(request: NativeRunRequest) -> Self {
        StartNativeRun {
            client_request_id: request.client_request_id,
            strict_mcp_config: request.strict_mcp_config,
            mcp_config_id: request.mcp_config_id,
            mcp_servers: request.mcp_servers,
            denied_shell_commands: request.denied_shell_commands,
            denied_mcp_tools: request.denied_mcp_tools,
            project_id: request.project_id,
            template_id: request.template_id,
            inputs: request.inputs,
            target: request.target,
            workspace: request.workspace,
            provider: request.provider,
            provider_by_role: request.provider_by_role,
            autonomous: request.autonomous,
            headless: request.headless,
            auto_approve_gates: request.auto_approve_gates,
            origin: None,
            model: request.model,
            permission_mode: request.permission_mode,
            effort: request.effort,
            ticket: None,
            vault: request.vault,
            no_vault: request.no_vault,
            agent_executor: AgentExecutor::default(),
        }
    }
}

/// Answer of `POST /api/runs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRunStarted {
    pub run_id: String,
}
