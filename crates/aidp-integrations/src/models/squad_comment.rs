use serde::{Deserialize, Serialize};

/// A recent comment on a squad pull request; the body is never kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadComment {
    pub id: String,
    pub author: String,
    pub created_at: String,
    pub body_marker: bool,
}
