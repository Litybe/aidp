use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::enums::permission_policy::PermissionPolicy;

/// Everything a provider needs to start one agent; `extra_args` is an opaque argv tail.
#[derive(Debug, Clone, Default)]
pub struct AgentSpec {
    pub work_dir: PathBuf,
    pub prompt: String,
    pub persona: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Reserved: no provider reads this field yet.
    pub allowed_tools: Vec<String>,
    /// Reserved: no provider reads this field yet.
    pub denied_tools: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub permission_policy: PermissionPolicy,
    /// Reserved: no provider reads this field yet.
    pub timeout: Option<Duration>,
    /// Reserved: no provider reads this field yet.
    pub budget_tokens: Option<u64>,
    pub extra_args: Vec<String>,
    pub session_id: Option<String>,
    pub strict_mcp_config: Option<bool>,
    pub mcp_servers: Option<Vec<serde_json::Value>>,
    pub mcp_configuration: Option<Arc<super::mcp_configuration::McpConfiguration>>,
    pub denied_shell_commands: Vec<String>,
    pub denied_mcp_tools: BTreeMap<String, Vec<String>>,
}
