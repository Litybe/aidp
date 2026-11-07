use crate::models::comment_point::CommentPoint;
use crate::pr::ReviewState;
use serde::Serialize;

/// One submitted review: its state, the commit it was made on and its body split into points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewSummary {
    pub id: Option<u64>,
    pub author: String,
    pub bot: bool,
    pub state: ReviewState,
    pub commit_sha: Option<String>,
    pub submitted_at: Option<String>,
    pub url: Option<String>,
    pub points: Vec<CommentPoint>,
}
