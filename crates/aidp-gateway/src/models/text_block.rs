use serde::Serialize;

use super::enums::TextBlockType;

/// A text content block in the stream-json user message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextBlock {
    #[serde(rename = "type")]
    pub block_type: TextBlockType,
    pub text: String,
}
