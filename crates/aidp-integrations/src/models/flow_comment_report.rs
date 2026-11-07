//! What `flow jira-comment` prints.

use serde::Serialize;

/// The identifier of the comment, or the path of the local report.
#[derive(Debug, Clone, Serialize)]
pub struct FlowCommentReport {
    pub comment_id: String,
}
