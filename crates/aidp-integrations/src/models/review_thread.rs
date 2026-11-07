use crate::models::enums::addressed::Addressed;
use crate::models::enums::last_by::LastBy;
use crate::models::thread_comment::ThreadComment;
use serde::Serialize;

/// A pull request review thread: the first-comment fields plus the full conversation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewThread {
    pub thread_id: Option<String>,
    pub reply_to_id: Option<String>,
    pub is_resolved: bool,
    pub is_outdated: Option<bool>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub author: Option<String>,
    pub created_at: Option<String>,
    pub body: Option<String>,
    #[serde(rename = "startLine")]
    pub start_line: Option<i64>,
    pub comments: Vec<ThreadComment>,
    #[serde(
        rename = "commentsTruncated",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub comments_truncated: bool,
    #[serde(rename = "lastBy")]
    pub last_by: LastBy,
    pub addressed: Addressed,
    #[serde(rename = "waitingOnReviewer")]
    pub waiting_on_reviewer: bool,
}
