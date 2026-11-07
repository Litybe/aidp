use std::path::PathBuf;

/// A project the index and the memories belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    /// Stable key stored in the `project` column.
    pub key: String,
    /// Directory holding the project files.
    pub root: PathBuf,
}
