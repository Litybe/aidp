use crate::models::enums::FlowExecutorMode;
use serde::{Deserialize, Serialize};

/// Launch policy applied to every pipeline step of one flow execution.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowExecutionPolicy {
    #[serde(default)]
    pub autonomous: bool,
    #[serde(default)]
    pub auto_approve_gates: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_mcp_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied_shell_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub denied_mcp_tools: std::collections::BTreeMap<String, Vec<String>>,
    /// Every session of the flow runs without a permission prompt: what is not allowed is refused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub headless: bool,
    /// Who runs the actions and probes. Absent means the gateway does.
    #[serde(default, skip_serializing_if = "FlowExecutorMode::is_internal")]
    pub executor: FlowExecutorMode,
    /// The caller also runs the agent sessions of the pipeline steps. Needs `executor: external`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub external_agents: bool,
}

/// Refuses host-run agent sessions without the external executor, before the request is persisted.
pub fn validate_external_agents(policy: Option<&FlowExecutionPolicy>) -> anyhow::Result<()> {
    let invalid = policy.is_some_and(|policy| {
        policy.external_agents && policy.executor != FlowExecutorMode::External
    });
    anyhow::ensure!(
        !invalid,
        crate::models::constants::flow_execution::EXTERNAL_AGENTS_NEED_EXTERNAL_EXECUTOR
    );
    Ok(())
}

/// Refuses unsupported definitions before request bodies can be persisted.
pub fn validate_mcp_configuration(servers: Option<&[serde_json::Value]>) -> anyhow::Result<()> {
    if servers.is_some_and(|servers| !servers.is_empty()) {
        anyhow::bail!(crate::models::constants::flow_execution::UNSUPPORTED_MCP_DEFINITIONS);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/flow_execution_policy_tests.rs"]
mod tests;
