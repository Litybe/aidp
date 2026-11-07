use serde::{Deserialize, Serialize};

/// A column of the squad inbox, declared in priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    NeedsAction,
    ReReview,
    FirstReview,
    Discussion,
    Reviewed,
    Drafts,
    Mine,
}
