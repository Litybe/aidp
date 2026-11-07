use crate::models::enums::diff_side::DiffSide;
use serde::Serialize;

/// Inline comment as GitHub expects it, inside a review or on its own.
#[derive(Debug, Serialize)]
pub struct ReviewCommentPayload<'a> {
    pub path: &'a str,
    pub line: u32,
    pub side: DiffSide,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<DiffSide>,
    pub body: &'a str,
}
