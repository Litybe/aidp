use crate::models::raw_thread_comments::RawThreadComments;
use serde::Deserialize;

/// A review thread node as returned by the GraphQL API.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawThreadNode {
    pub id: Option<String>,
    #[serde(default)]
    pub is_resolved: bool,
    pub is_outdated: Option<bool>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub start_line: Option<i64>,
    #[serde(default)]
    pub comments: RawThreadComments,
}
