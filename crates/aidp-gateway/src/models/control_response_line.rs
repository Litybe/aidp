use serde::Serialize;

use super::control_response_body::ControlResponseBody;
use super::enums::StreamMessageType;

/// A stream-json control response line written to `claude`'s stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ControlResponseLine {
    #[serde(rename = "type")]
    pub message_type: StreamMessageType,
    pub response: ControlResponseBody,
}
