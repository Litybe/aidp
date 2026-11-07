use serde::{Deserialize, Serialize};

/// Body of `POST /api/prs/fix`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PrFixRequest {
    pub repo: String,
    pub number: u64,
    pub threads: Vec<String>,
    #[serde(default)]
    pub points: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

#[cfg(test)]
#[path = "../../tests/unit/pr_fix_models_tests.rs"]
mod tests;
