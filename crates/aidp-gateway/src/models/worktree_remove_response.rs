use serde::{Deserialize, Serialize};

/// Reply of `POST /api/worktrees/remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRemoveResponse {
    pub removed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salvaged_ref: Option<String>,
}
