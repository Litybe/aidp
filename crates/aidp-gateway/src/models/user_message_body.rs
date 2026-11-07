use serde::Serialize;

use super::enums::MessageRole;
use super::reply_content::ReplyContent;

/// The `message` of a stream-json user line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserMessageBody {
    pub role: MessageRole,
    pub content: ReplyContent,
}
