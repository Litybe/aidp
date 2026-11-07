use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::enums::WorkspaceKind;
use crate::worktrees::normalize_path;

/// Where a session runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub kind: WorkspaceKind,
    pub path: PathBuf,
    pub branch: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

impl Workspace {
    /// The admission key: the normalized path, so two spellings of one directory collide.
    pub fn key(&self) -> String {
        normalize_path(&self.path)
    }
}
