use aidp_integrations::models::enums::review_event::ReviewEvent;
use serde::Serialize;

/// Payload of `pr.review_submitted`, published once per submitted review.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewSubmittedEvent {
    pub repo: String,
    pub number: u64,
    pub review_id: u64,
    pub event: ReviewEvent,
    pub comment_count: usize,
}
