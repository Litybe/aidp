use crate::pr::ReviewState;
use serde::{Deserialize, Serialize};

/// One review left on a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SquadReview {
    pub author: String,
    pub state: ReviewState,
    pub commit_oid: Option<String>,
    pub submitted_at: Option<String>,
}
