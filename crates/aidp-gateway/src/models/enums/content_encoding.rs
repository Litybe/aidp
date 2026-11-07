use crate::models::constants::static_ui::{BROTLI_SUFFIX, BROTLI_TOKEN, GZIP_SUFFIX, GZIP_TOKEN};

/// A precompressed variant of a UI file, written next to it by the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentEncoding {
    Brotli,
    Gzip,
}

impl ContentEncoding {
    /// Server preference order: the smallest variant first.
    pub const PREFERENCE: [ContentEncoding; 2] = [ContentEncoding::Brotli, ContentEncoding::Gzip];

    /// The `Accept-Encoding` and `Content-Encoding` token.
    pub fn token(self) -> &'static str {
        match self {
            ContentEncoding::Brotli => BROTLI_TOKEN,
            ContentEncoding::Gzip => GZIP_TOKEN,
        }
    }

    /// The file name suffix of the variant, `.br` or `.gz`.
    pub fn suffix(self) -> &'static str {
        match self {
            ContentEncoding::Brotli => BROTLI_SUFFIX,
            ContentEncoding::Gzip => GZIP_SUFFIX,
        }
    }
}
