use super::rest_comment_user::RestCommentUser;
use serde::Serialize;

/// One comment rebuilt in the REST shape the comment parser reads.
#[derive(Debug, Serialize)]
pub struct RestCommentPage {
    pub id: u64,
    pub user: RestCommentUser,
    pub body: String,
    #[serde(rename = "html_url")]
    pub html_url: String,
    #[serde(rename = "created_at")]
    pub created_at: String,
}
