use serde::Serialize;

/// Where the GitHub environment approval of a deployment stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowApprovalState {
    NotRequired,
    Approved,
    Pending,
}
