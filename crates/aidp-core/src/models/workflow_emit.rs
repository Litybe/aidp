use crate::plan;

/// The result of compiling a run.
#[derive(Debug)]
pub struct Emitted {
    pub script: String,
    pub plan_schedules: Vec<plan::ScheduleState>,
    /// Set when the script stops at a `plan_tasks:` stage awaiting its plan.
    pub awaiting_plan: Option<AwaitingPlan>,
    /// Set when the script stops right before a composite (`type: pipeline`) entry stage that has
    /// not yet been crossed.
    pub awaiting_composite: Option<AwaitingComposite>,
    /// Names of every stage `is_gate()` found true while compiling this pass, in pipeline order.
    pub gates: Vec<String>,
    /// One message per `when:`/`gate:` expression this pass could not parse (see
    /// `expr::eval_when_collect`/`eval_loop_when_collect`/`eval_gate_collect`), in evaluation order.
    pub warnings: Vec<String>,
}

/// The real `PlanSource`: reads `plan.json` from the workspace.
pub struct DiskPlanSource;

/// The `plan_tasks:` stage the compilation stopped at, and the artifact it waits for.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AwaitingPlan {
    pub stage: String,
    pub producer: String,
    pub plan_path: String,
}

/// The composite (`type: pipeline`) entry stage the compilation stopped before, forcing a new pass.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AwaitingComposite {
    pub stage: String,
    pub composite: String,
}
