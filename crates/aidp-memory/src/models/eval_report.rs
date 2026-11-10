use crate::models::eval_miss::EvalMiss;
use serde::{Deserialize, Serialize};

/// Result of `rag eval`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalReport {
    pub cases: usize,
    pub k: usize,
    /// Share of the cases answered within the first `k` hits.
    pub recall: f64,
    /// Mean reciprocal rank of the first answering hit, over the wider search.
    pub mrr: f64,
    pub misses: Vec<EvalMiss>,
}
