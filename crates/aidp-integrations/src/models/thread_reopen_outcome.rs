use serde::Serialize;

/// Result of replying on a review thread and reopening it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReopenOutcome {
    pub thread_id: String,
    pub replied: bool,
    pub unresolved: bool,
}
