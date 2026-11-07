pub const HTTP: &str = "http";
pub const CONFIGURATION_ERROR: &str = "MCP provider configuration failed";
pub const PROVIDER_STDERR: &str = "agent stderr";
pub const CONFIG_FLAG: &str = "--mcp-config";
pub const REDACTED_VALUE: &str = "<redacted>";
pub const CLAUDE_VALUE_PREFIX: &str = "AIDP_MCP_VALUE_";
pub const CONTROL_FIELDS: &[&str] = &[
    "type",
    "subtype",
    "session_id",
    "model",
    "permissionMode",
    "id",
    "name",
    "request_id",
    "tool_name",
    "tool_use_id",
    "role",
    "tools",
];
pub const PAYLOAD_FIELDS: &[&str] = &["input", "output", "questions"];
