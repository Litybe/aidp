use serde::{Deserialize, Serialize};

/// Body of `POST /api/prs/viewed`: marks or unmarks one file as viewed.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrViewedRequest {
    pub repo: String,
    pub number: u64,
    pub path: String,
    pub viewed: bool,
}
