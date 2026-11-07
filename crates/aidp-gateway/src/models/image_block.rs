use serde::Serialize;

use super::enums::ImageBlockType;
use super::image_source::ImageSource;

/// An image content block in the stream-json user message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageBlock {
    #[serde(rename = "type")]
    pub block_type: ImageBlockType,
    pub source: ImageSource,
}
