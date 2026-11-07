use serde::{Deserialize, Serialize};

/// Final JSON summary line a PR-fix session prints.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct PrFixSummary {
    #[serde(default)]
    pub fixed: Vec<String>,
    #[serde(default)]
    pub needs_reviewer: Vec<String>,
    #[serde(default)]
    pub not_relevant: Vec<String>,
    #[serde(default)]
    pub pushed_sha: Option<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
}

impl PrFixSummary {
    /// The selected threads the summary places in no bucket.
    pub fn unreported(&self, selected: &[String]) -> Vec<String> {
        selected
            .iter()
            .filter(|id| {
                ![&self.fixed, &self.needs_reviewer, &self.not_relevant]
                    .iter()
                    .any(|bucket| bucket.contains(id))
            })
            .cloned()
            .collect()
    }
}
