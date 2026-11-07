use serde::Deserialize;

/// Body of `POST /api/prs/marks`.
#[derive(Debug, Deserialize)]
pub struct PrMarkRequest {
    pub repo: String,
    pub number: u64,
    #[serde(default)]
    pub seen: Option<bool>,
    #[serde(default)]
    pub ignored: Option<bool>,
}
