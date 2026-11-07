use serde::{Deserialize, Serialize};

/// Machine-read reason carried beside a refused PR route's message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrRefusalReason {
    OwnPr,
    NoPullRequest,
    NotMine,
}
