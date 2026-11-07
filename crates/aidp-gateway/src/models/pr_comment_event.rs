use aidp_integrations::models::enums::comment_kind::CommentKind;
use serde::Serialize;

/// Payload of `pr.comment_created`, `pr.comment_updated` and `pr.comment_deleted`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCommentEvent {
    pub repo: String,
    pub number: u64,
    pub comment_id: u64,
    pub kind: CommentKind,
}
