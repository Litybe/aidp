use crate::models::raw_author::RawAuthor;
use serde::Deserialize;

/// A review-thread comment as returned by the GraphQL API.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawThreadComment {
    pub id: Option<String>,
    pub database_id: Option<i64>,
    pub author: Option<RawAuthor>,
    pub body: Option<String>,
    pub created_at: Option<String>,
    pub url: Option<String>,
    #[serde(rename = "diffHunk", default)]
    pub diff_hunk: Option<String>,
}
