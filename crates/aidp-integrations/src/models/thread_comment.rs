use serde::Serialize;

/// One comment of a review thread, with whether the viewer wrote it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadComment {
    pub id: Option<String>,
    #[serde(rename = "databaseId")]
    pub database_id: Option<i64>,
    pub author: Option<String>,
    #[serde(rename = "isMine")]
    pub is_mine: bool,
    pub body: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    pub url: Option<String>,
    pub diff_hunk: Option<String>,
}
