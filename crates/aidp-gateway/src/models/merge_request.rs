use serde::Deserialize;

/// Body of `POST /api/prs/merge`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequest {
    pub repo: String,
    pub number: u64,
    pub expected_head_sha: String,
}
