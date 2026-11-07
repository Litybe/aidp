pub const EXECUTION_KEY: &str = "executionKey";
pub const TICKET_KEY: &str = "key";
pub const TICKET_DESCRIPTION: &str = "description";
pub const TICKET_PROJECT: &str = "project";
pub const TICKET_SERVICE: &str = "service";
pub const TICKET_TYPE: &str = "type";
pub const EMPTY_CLIENT_REQUEST_ID: &str = "clientRequestId must not be blank";
pub const EXTERNAL_AGENTS_NEED_EXTERNAL_EXECUTOR: &str =
    "externalAgents requires executor external";
pub const UNSUPPORTED_MCP_DEFINITIONS: &str = "MCP server definitions are not supported; provide an empty mcpServers list with strictMcpConfig enabled";
pub const GATEWAY_URL_ENV: &str = "AIDP_GATEWAY_URL";
pub const GATEWAY_DIR_ENV: &str = "AIDP_GATEWAY_DIR";
pub const UI_TOKEN_ENV: &str = "AIDP_UI_TOKEN";
pub const VAULT_ENV: &str = "AIDP_VAULT";
pub const VAULT_DISABLED: &str = "off";
pub const AIDP_BIN_ENV: &str = "AIDP_BIN";
pub const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";
pub const LOOPBACK_HTTP: &str = "http://127.0.0.1";
pub const LOOPBACK_HTTP_V6: &str = "http://[::1]";
pub const EMPTY_ACTION_ERROR: &str =
    "command failed without structured output; check integration configuration and host logs";
