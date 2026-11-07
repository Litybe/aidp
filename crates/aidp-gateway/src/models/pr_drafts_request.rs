use aidp_integrations::models::review_draft::ReviewDraft;
use serde::{Deserialize, Serialize};

/// Body of `PUT /api/prs/drafts`: replaces every draft of the pull request.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDraftsRequest {
    pub repo: String,
    pub number: u64,
    pub drafts: Vec<ReviewDraft>,
}
