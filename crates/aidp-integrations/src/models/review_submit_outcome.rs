use serde::Serialize;

/// Result of submitting a review with all its inline comments in one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSubmitOutcome {
    pub id: u64,
    pub state: String,
    pub url: Option<String>,
    pub comment_count: usize,
}
