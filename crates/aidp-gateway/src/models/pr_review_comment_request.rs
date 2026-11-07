use aidp_integrations::models::enums::diff_side::DiffSide;
use serde::{Deserialize, Serialize};

/// Body of `POST /api/prs/review-comments`: an inline comment anchored on a diff line.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewCommentRequest {
    pub repo: String,
    pub number: u64,
    pub path: String,
    pub line: u32,
    pub side: DiffSide,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub start_side: Option<DiffSide>,
    pub body: String,
}
