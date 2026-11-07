use serde::Deserialize;

/// Author of a GitHub REST comment.
#[derive(Debug, Deserialize)]
pub struct RawUser {
    #[serde(default)]
    pub login: String,
}
