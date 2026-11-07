//! The port through which the GitHub sync reads issues and writes their state labels.
//!
//! The ticket domain defines [`GithubPort`]; `aidp-integrations` implements it over the `gh` CLI
//! and tests hand in fakes. `Sync` because the sync reads dependencies on several threads.

use crate::models::github_issue::{GithubIssue, IssueDependency};

/// Reads issues of a repository (`owner/name`) and edits their labels and comments.
pub trait GithubPort: Send + Sync {
    /// Open issues, newest first, at most `limit` of them.
    fn list_open(&self, repo: &str, limit: u32) -> anyhow::Result<Vec<GithubIssue>>;

    /// One issue, open or closed; `None` when it does not exist.
    fn view(&self, repo: &str, number: u64) -> anyhow::Result<Option<GithubIssue>>;

    /// The issues GitHub's native dependencies list as blocking `number`.
    fn blocked_by(&self, repo: &str, number: u64) -> anyhow::Result<Vec<IssueDependency>>;

    /// Adds then removes labels on issue `number`.
    fn edit_labels(
        &self,
        repo: &str,
        number: u64,
        add: &[String],
        remove: &[String],
    ) -> anyhow::Result<()>;

    /// Posts a comment on issue `number`, returning its URL when GitHub gives one.
    fn comment(&self, repo: &str, number: u64, body: &str) -> anyhow::Result<Option<String>>;
}
