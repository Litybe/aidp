use crate::models::raw_thread_comment::RawThreadComment;
use serde::Deserialize;

/// The comment connection of a review thread.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawThreadComments {
    #[serde(default)]
    pub nodes: Vec<RawThreadComment>,
    #[serde(default)]
    pub total_count: i64,
}
