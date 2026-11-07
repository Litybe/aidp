use serde::Serialize;

use super::image_block::ImageBlock;
use super::text_block::TextBlock;

/// One block of a user message that carries images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ContentBlock {
    Text(TextBlock),
    Image(ImageBlock),
}
