use serde::{Deserialize, Serialize};

/// The closed set of `kind` values a session event carries, written by
/// [`crate::sessions`] and read back by its transcript renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    WorkflowProgress,
    System,
    Text,
    Thinking,
    Question,
    ToolUse,
    ToolResult,
    TurnEnd,
    Raw,
    Log,
    Error,
    UserReply,
    UserMessage,
}

impl SessionEventKind {
    /// The wire value serde writes for this variant.
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionEventKind::WorkflowProgress => "workflow_progress",
            SessionEventKind::System => "system",
            SessionEventKind::Text => "text",
            SessionEventKind::Thinking => "thinking",
            SessionEventKind::Question => "question",
            SessionEventKind::ToolUse => "tool_use",
            SessionEventKind::ToolResult => "tool_result",
            SessionEventKind::TurnEnd => "turn_end",
            SessionEventKind::Raw => "raw",
            SessionEventKind::Log => "log",
            SessionEventKind::Error => "error",
            SessionEventKind::UserReply => "user_reply",
            SessionEventKind::UserMessage => "user_message",
        }
    }
}
