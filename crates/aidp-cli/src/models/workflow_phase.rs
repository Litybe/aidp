use serde::Serialize;

/// Stderr line printed when emit stops before a plan-driven stage.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct AwaitingPlanPhase<'a> {
    pub note: &'a str,
    pub phase: &'a str,
    pub plan_path: &'a str,
    pub producer: &'a str,
    pub stage: &'a str,
    pub warnings: &'a [String],
}

/// Per-plan summary carried by the `plan_compiled` phase line.
#[derive(Serialize)]
pub struct PlanSummary<'a> {
    pub max_parallel: u32,
    pub serializations: usize,
    pub stage: &'a str,
    pub tasks: usize,
    pub warnings: usize,
    pub waves: usize,
}

/// Stderr line printed once a plan's tasks have been compiled.
#[derive(Serialize)]
pub struct PlanCompiledPhase<'a> {
    pub phase: &'a str,
    pub plans: Vec<PlanSummary<'a>>,
    pub warnings: &'a [String],
}

/// Stderr line printed when emit stops before a composite stage.
#[derive(Serialize)]
pub struct AwaitingCompositePhase<'a> {
    pub composite: &'a str,
    pub note: &'a str,
    pub phase: &'a str,
    pub stage: &'a str,
    pub warnings: &'a [String],
}
