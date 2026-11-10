use serde::{Deserialize, Serialize};

/// A case whose answer was not among the first `k` hits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalMiss {
    pub query: String,
    pub expect: Vec<String>,
    /// Rank of the first answering hit beyond `k`, if any came back.
    pub rank: Option<usize>,
    pub top: Vec<String>,
}
