use crate::models::enums::content_encoding::ContentEncoding;

/// A file of the UI build, read and ready to send, possibly its precompressed variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticFile {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub cache_control: &'static str,
    pub content_encoding: Option<ContentEncoding>,
}
