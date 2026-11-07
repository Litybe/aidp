use crate::models::posted_thread::PostedThread;
use serde::Serialize;

/// Result of posting the findings of one pass as a single review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPostOutcome {
    pub posted: bool,
    pub review_id: Option<u64>,
    pub threads: Vec<PostedThread>,
}
