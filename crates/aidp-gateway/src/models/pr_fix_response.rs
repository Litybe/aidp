use crate::models::enums::workspace_kind::WorkspaceKind;
use serde::{Deserialize, Serialize};

/// Reply of `POST /api/prs/fix`; `dropped_threads` were already resolved on GitHub.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrFixResponse {
    pub session_id: String,
    pub workspace: String,
    pub workspace_kind: WorkspaceKind,
    pub threads: Vec<String>,
    pub dropped_threads: Vec<String>,
}
