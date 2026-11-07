use crate::pr::{Mergeable, ReviewDecision, StatusContextState};

/// An open pull request as the light listing reads it: enough to tell whether the
/// cached details are still current, plus the states that change without an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquadListedPr {
    pub repo: String,
    pub number: u64,
    pub updated_at: String,
    pub head_sha: String,
    pub is_draft: bool,
    pub mergeable: Mergeable,
    pub review_decision: Option<ReviewDecision>,
    pub checks_state: Option<StatusContextState>,
}
