use serde::Deserialize;

/// Body of `POST /api/prs/threads/reply`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReplyRequest {
    pub repo: String,
    pub number: u64,
    pub thread_id: String,
    pub body: String,
}
