use serde::Deserialize;

/// Body of `POST /api/prs/ready`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyRequest {
    pub repo: String,
    pub number: u64,
    pub draft: bool,
}
