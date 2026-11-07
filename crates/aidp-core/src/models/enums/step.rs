use super::fanout_items::FanoutItems;
use super::skip_reason::SkipReason;
use crate::models::stage_spec::StageSpec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One unit of a pass, in execution order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    Stage(StageSpec),
    /// Adjacent independent stages dispatched together.
    Parallel {
        stages: Vec<StageSpec>,
    },
    /// A correction loop. `condition` is the `stages.<s>.outputs.<n>.<p>` reference, `None` when
    /// unconditional. `body_producer` is the body stage that refreshes the source each iteration.
    Loop {
        container: String,
        condition: Option<String>,
        source_stage: Option<String>,
        signal: Option<String>,
        body_producer: Option<String>,
        max: u32,
        body: Vec<Step>,
    },
    Fanout {
        stage: StageSpec,
        items: FanoutItems,
        max: u32,
        fold: bool,
        merge_hint: Option<String>,
    },
    /// A plan-driven stage. When its plan is missing the pass ends here with `AwaitingPlan`.
    PlanTasks {
        stage: StageSpec,
        from: String,
        plan_path: String,
        max: u32,
        kinds: Vec<String>,
        merge_hint: Option<String>,
        skills_by_kind: BTreeMap<String, Vec<String>>,
    },
    /// The pass stops here, before the composite named `composite`.
    CompositeBoundary {
        stage: String,
        composite: String,
    },
    Skipped {
        stage: String,
        reason: SkipReason,
    },
}
