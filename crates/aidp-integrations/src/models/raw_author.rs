use serde::Deserialize;

/// Author of a GraphQL review-thread comment.
#[derive(Debug, Default, Deserialize)]
pub struct RawAuthor {
    pub login: Option<String>,
}
