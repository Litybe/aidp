use serde::{Deserialize, Serialize};

/// Why a pass stops before the end of the pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PassEnd {
    /// A `plan_tasks:` stage whose plan is not on disk yet.
    AwaitingPlan {
        stage: String,
        producer: String,
        plan_path: String,
    },
    /// A composite entry that would overflow the pass budget.
    AwaitingComposite { stage: String, composite: String },
}
