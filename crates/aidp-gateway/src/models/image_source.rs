use serde::Serialize;

use super::enums::{ImageMediaType, ImageSourceType};

/// The base64 payload of an image block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: ImageSourceType,
    pub media_type: ImageMediaType,
    pub data: String,
}
