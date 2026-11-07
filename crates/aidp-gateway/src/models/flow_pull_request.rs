use serde::Serialize;

/// The pull request a flow run opened, as the flow view exposes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowPullRequest {
    pub number: u64,
    pub url: Option<String>,
    /// `owner/name`.
    pub repository: Option<String>,
}
