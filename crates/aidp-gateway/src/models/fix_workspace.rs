use crate::models::enums::WorkspaceKind;
use crate::models::WorkspaceRequest;
use std::path::PathBuf;

/// Where a fix session runs and who owns the directory.
pub struct FixWorkspace {
    pub request: WorkspaceRequest,
    pub path: PathBuf,
    pub kind: WorkspaceKind,
    pub owned: bool,
}
