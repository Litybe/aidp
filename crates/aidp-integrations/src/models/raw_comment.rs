use crate::models::raw_user::RawUser;
use serde::Deserialize;

/// A GitHub REST comment or review as returned by the API.
#[derive(Debug, Deserialize)]
pub struct RawComment {
    pub id: u64,
    pub user: Option<RawUser>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default, alias = "submitted_at")]
    pub created_at: String,
}
