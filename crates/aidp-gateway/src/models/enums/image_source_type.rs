use serde::Serialize;

/// How the bytes of an image block are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageSourceType {
    Base64,
}
