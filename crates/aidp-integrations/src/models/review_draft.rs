use crate::models::enums::diff_side::DiffSide;
use serde::{Deserialize, Serialize};

/// An inline comment queued locally until the review is submitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDraft {
    pub id: String,
    pub path: String,
    pub line: u32,
    pub side: DiffSide,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub start_side: Option<DiffSide>,
    pub body: String,
}
