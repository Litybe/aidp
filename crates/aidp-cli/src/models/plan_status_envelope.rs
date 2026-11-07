use serde::Serialize;

use super::plan_task_status_count::PlanTaskStatusCount;

/// Envelope printed by `aidp workflow plan-status`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct PlanStatusEnvelope<'a, Awaiting: Serialize, Schedules: Serialize> {
    pub awaiting_plan: &'a Option<Awaiting>,
    pub compiled: bool,
    pub pipeline_id: &'a str,
    pub plan_schedules: &'a [Schedules],
    pub run_id: &'a str,
    pub task_status_counts: Vec<PlanTaskStatusCount<'a>>,
}
