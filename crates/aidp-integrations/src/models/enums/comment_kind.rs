use serde::{Deserialize, Serialize};

/// Where a global PR comment comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentKind {
    Conversation,
    Review,
}
