use serde::Deserialize;

/// Body of `POST /api/prs/threads/resolve`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsResolveRequest {
    pub repo: String,
    pub number: u64,
    pub thread_ids: Vec<String>,
    #[serde(default)]
    pub unresolve: bool,
}
