use crate::models::enums::review_event::ReviewEvent;
use crate::models::review_comment_payload::ReviewCommentPayload;
use serde::Serialize;

/// Body of the single request that submits a review with all its comments.
#[derive(Debug, Serialize)]
pub struct ReviewPayload<'a> {
    pub event: ReviewEvent,
    pub body: &'a str,
    pub commit_id: &'a str,
    pub comments: Vec<ReviewCommentPayload<'a>>,
}
