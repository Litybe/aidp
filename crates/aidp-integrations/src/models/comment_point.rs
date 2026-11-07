use serde::{Deserialize, Serialize};

/// One actionable chunk of a comment, cut by heading or list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentPoint {
    /// `<commentId>#<index>`; absent until the point is attached to a comment.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<String>,
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    pub body: String,
}
