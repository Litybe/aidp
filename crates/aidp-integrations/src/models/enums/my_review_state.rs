use serde::{Deserialize, Serialize};

/// What my latest review of a pull request amounts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MyReviewState {
    None,
    Commented,
    Approved,
    ChangesRequested,
    Stale,
    Dismissed,
}
