use serde::Serialize;

/// Author object of a REST-shaped comment.
#[derive(Debug, Serialize)]
pub struct RestCommentUser {
    pub login: String,
}
