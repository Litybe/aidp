//! A file a GitHub commit adds or replaces.

use serde::Serialize;

/// A repository path and its whole content, base64 encoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileAddition {
    pub path: String,
    pub contents: String,
}
