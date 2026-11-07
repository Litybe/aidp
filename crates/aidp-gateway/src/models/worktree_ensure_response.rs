use serde::{Deserialize, Serialize};

/// Reply of `POST /api/worktrees/ensure`: the worktree, or the reason the step must wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeEnsureResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha: Option<String>,
    pub reused: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waiting_external: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
