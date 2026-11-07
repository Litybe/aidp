use crate::models::global_comment::GlobalComment;
use crate::models::pr_commit::PrCommit;
use crate::models::pr_review_summary::PrReviewSummary;
use crate::models::review_thread::ReviewThread;
use crate::pr::PrStatus;
use serde::Serialize;

/// Everything a pull request page shows, read in one logical request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDetail {
    pub status: PrStatus,
    pub description: String,
    pub commits: Vec<PrCommit>,
    pub reviews: Vec<PrReviewSummary>,
    pub comments: Vec<GlobalComment>,
    pub threads: Vec<ReviewThread>,
    pub requested_reviewers: Vec<String>,
}
