use serde::Serialize;

use super::content_block::ContentBlock;

/// Message content: the bare text, or a text block followed by the image blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ReplyContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}
