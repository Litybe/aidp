use serde::{Deserialize, Serialize};

/// The `type` field of a stream-json message exchanged with `claude`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamMessageType {
    /// A message originating from the user (a prompt, a reply, an answer).
    User,
    /// A message produced by the assistant.
    Assistant,
    /// The final result of a turn, carrying usage.
    Result,
    /// A request from `claude` to the host, such as a tool permission.
    ControlRequest,
    /// The host's answer to a control request.
    ControlResponse,
}

impl StreamMessageType {
    /// The wire value serde writes for this variant.
    pub const fn as_str(self) -> &'static str {
        match self {
            StreamMessageType::User => "user",
            StreamMessageType::Assistant => "assistant",
            StreamMessageType::Result => "result",
            StreamMessageType::ControlRequest => "control_request",
            StreamMessageType::ControlResponse => "control_response",
        }
    }
}
