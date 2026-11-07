use serde::Serialize;

/// Comment as returned by GitHub after a create or an edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentWriteOutcome {
    pub id: u64,
    pub url: Option<String>,
}
