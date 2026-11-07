use super::exit_reason::ExitReason;
use crate::models::token_usage::TokenUsage;

/// One normalized event streamed from a running agent.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        id: String,
        output: serde_json::Value,
        is_error: bool,
    },
    Usage(TokenUsage),
    PermissionRequest {
        id: String,
        tool: String,
        input: serde_json::Value,
    },
    Raw(serde_json::Value),
    Exited {
        code: Option<i32>,
        reason: ExitReason,
    },
}
