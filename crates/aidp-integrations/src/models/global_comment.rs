use crate::models::comment_point::CommentPoint;
use crate::models::enums::comment_kind::CommentKind;
use serde::{Deserialize, Serialize};

/// A conversation comment or review body of a PR, split into points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalComment {
    pub id: u64,
    pub author: String,
    pub bot: bool,
    pub created_at: String,
    pub url: String,
    pub kind: CommentKind,
    pub points: Vec<CommentPoint>,
}
