use std::path::PathBuf;

/// Project a ticket belongs to, as the executor needs it.
#[derive(Debug, Clone)]
pub struct ProjectRef {
    pub id: String,
    pub path: PathBuf,
    pub repo: String,
}
