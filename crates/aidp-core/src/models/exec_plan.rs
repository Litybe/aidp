use crate::models::enums::pass_end::PassEnd;
use crate::models::enums::step::Step;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The compiled shape of one Workflow pass: ordered steps, gates, warnings and how the pass ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecPlan {
    pub run_id: String,
    pub template: String,
    pub steps: Vec<Step>,
    pub warnings: Vec<String>,
    pub gates: Vec<String>,
    pub end: Option<PassEnd>,
    /// Stages a correction loop repairs, so their `needs_review` never pauses the run.
    #[serde(skip)]
    pub reviewed_stages: BTreeSet<String>,
}
