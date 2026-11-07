use serde::Serialize;

use super::enums::StreamMessageType;
use super::user_message_body::UserMessageBody;

/// A stream-json line carrying a user message, written to `claude`'s stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserStreamMessage {
    #[serde(rename = "type")]
    pub message_type: StreamMessageType,
    pub message: UserMessageBody,
}
