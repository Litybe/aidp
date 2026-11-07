use serde::Serialize;

/// The content block kind of an image block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageBlockType {
    Image,
}
