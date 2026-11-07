use crate::models::review_comment_payload::ReviewCommentPayload;
use serde::Serialize;

/// Body of a standalone inline comment: the anchor plus the commit it targets.
#[derive(Debug, Serialize)]
pub struct InlineCommentPayload<'a> {
    pub commit_id: &'a str,
    #[serde(flatten)]
    pub comment: ReviewCommentPayload<'a>,
}
