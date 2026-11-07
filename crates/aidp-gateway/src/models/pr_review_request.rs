use aidp_integrations::models::enums::review_event::ReviewEvent;
use serde::{Deserialize, Serialize};

/// Body of `POST /api/prs/review`: the verdict and summary sent with the stored drafts.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewRequest {
    pub repo: String,
    pub number: u64,
    pub event: ReviewEvent,
    #[serde(default)]
    pub body: String,
}
