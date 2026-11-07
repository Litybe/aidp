//! The files a GitHub commit changes.

use crate::models::file_addition::FileAddition;
use crate::models::file_deletion::FileDeletion;
use serde::Serialize;

/// Added or replaced files, then deleted ones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FileChanges {
    pub additions: Vec<FileAddition>,
    pub deletions: Vec<FileDeletion>,
}

impl FileChanges {
    /// True when the commit would change nothing.
    pub fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.deletions.is_empty()
    }
}
