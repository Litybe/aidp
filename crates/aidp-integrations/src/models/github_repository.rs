//! A GitHub repository named by its owner and name.

use serde::Serialize;

/// `owner/name` of a repository; serialized as the variables of the repository query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GithubRepository {
    pub owner: String,
    pub name: String,
}

impl GithubRepository {
    /// The `owner/name` spelling.
    pub fn name_with_owner(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}
