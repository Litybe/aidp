use serde::{Deserialize, Serialize};

/// Body of the worktree ensure and remove routes: the ticket whose worktree is meant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeKeyRequest {
    pub key: String,
}
