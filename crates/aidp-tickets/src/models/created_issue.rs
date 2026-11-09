use crate::models::enums::issue_state::IssueState;

/// An issue GitHub has just created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedIssue {
    pub number: u64,
    /// Database id, the handle the sub-issue API wants (not the issue number).
    pub id: u64,
    pub url: String,
}

/// One sub-issue of an issue, open or closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubIssue {
    pub number: u64,
    pub state: IssueState,
    pub labels: Vec<String>,
}
