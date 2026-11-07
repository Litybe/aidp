use serde::{Deserialize, Serialize};

use crate::models::enums::WorkspaceKind;

/// Where a session asks to run, before it is resolved to a concrete directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRequest {
    pub kind: WorkspaceKind,
    #[serde(default)]
    pub ticket: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}
