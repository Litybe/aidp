use crate::models::hit::Hit;
use serde::{Deserialize, Serialize};

/// Hits kept under a token budget.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Packed {
    pub hits: Vec<Hit>,
    pub used_tokens: usize,
    pub budget_tokens: usize,
    /// Hits dropped for lack of budget.
    pub dropped: usize,
}
