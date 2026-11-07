use serde::{Deserialize, Serialize};

/// Body of a comment create or edit: the target pull request and the new text.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCommentRequest {
    pub repo: String,
    pub number: u64,
    pub body: String,
}
