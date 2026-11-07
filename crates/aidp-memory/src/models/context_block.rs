use crate::models::hit::Hit;
use crate::models::memory::Memory;
use serde::{Deserialize, Serialize};

/// The Markdown block injected into a stage, with what it contains.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBlock {
    pub markdown: String,
    pub memories: Vec<Memory>,
    pub excerpts: Vec<Hit>,
    pub used_tokens: usize,
    pub budget_tokens: usize,
}
