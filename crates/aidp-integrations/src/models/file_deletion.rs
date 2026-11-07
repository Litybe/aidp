//! A file a GitHub commit deletes.

use serde::Serialize;

/// The repository path of the deleted file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDeletion {
    pub path: String,
}
