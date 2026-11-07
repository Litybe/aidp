use serde::Deserialize;

/// Body of `POST /api/prs/update-branch`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBranchRequest {
    pub repo: String,
    pub number: u64,
}
