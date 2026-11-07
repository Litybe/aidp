use serde::Serialize;

/// Result of a thread reply.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRepliedResult {
    pub thread_id: String,
    pub replied: bool,
}
