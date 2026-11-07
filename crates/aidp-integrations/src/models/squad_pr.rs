use crate::models::squad_comment::SquadComment;
use crate::models::squad_review::SquadReview;
use crate::pr::{Mergeable, ReviewDecision, StatusContextState};
use serde::{Deserialize, Serialize};

/// An open pull request of a squad repository, as the inbox sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SquadPr {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub is_draft: bool,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
    pub base_ref: String,
    pub head_ref: String,
    pub head_sha: String,
    pub mergeable: Mergeable,
    pub review_decision: Option<ReviewDecision>,
    pub review_requests: Vec<String>,
    pub my_reviews: Vec<SquadReview>,
    pub commits: u64,
    pub checks_state: Option<StatusContextState>,
    pub unresolved_threads: u64,
    pub comments: u64,
    pub labels: Vec<String>,
    #[serde(default)]
    pub recent_comments: Vec<SquadComment>,
}
