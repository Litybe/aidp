use crate::models::pr_flow_ref::PrFlowRef;
use aidp_integrations::models::enums::lane::Lane;
use aidp_integrations::models::enums::my_review_state::MyReviewState;
use aidp_integrations::models::squad_pr::SquadPr;
use aidp_integrations::pr::{Mergeable, ReviewDecision, StatusContextState};
use serde::Serialize;

/// One inbox row: a squad pull request with its lanes and local marks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadPrRow {
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
    pub commits: u64,
    pub checks_state: Option<StatusContextState>,
    pub unresolved_threads: u64,
    pub comments: u64,
    pub labels: Vec<String>,
    pub lanes: Vec<Lane>,
    pub my_review_state: MyReviewState,
    pub seen: bool,
    pub ignored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<PrFlowRef>,
}

impl SquadPrRow {
    pub fn new(
        pr: &SquadPr,
        lanes: Vec<Lane>,
        my_review_state: MyReviewState,
        seen: bool,
        ignored: bool,
    ) -> Self {
        SquadPrRow {
            repo: pr.repo.clone(),
            number: pr.number,
            title: pr.title.clone(),
            url: pr.url.clone(),
            is_draft: pr.is_draft,
            author: pr.author.clone(),
            created_at: pr.created_at.clone(),
            updated_at: pr.updated_at.clone(),
            base_ref: pr.base_ref.clone(),
            head_ref: pr.head_ref.clone(),
            head_sha: pr.head_sha.clone(),
            mergeable: pr.mergeable,
            review_decision: pr.review_decision,
            review_requests: pr.review_requests.clone(),
            commits: pr.commits,
            checks_state: pr.checks_state,
            unresolved_threads: pr.unresolved_threads,
            comments: pr.comments,
            labels: pr.labels.clone(),
            lanes,
            my_review_state,
            seen,
            ignored,
            flow: None,
        }
    }
}
