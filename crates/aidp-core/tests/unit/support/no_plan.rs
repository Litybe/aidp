use crate::models::port::PlanSource;
use crate::plan::Plan;
use std::path::Path;

/// A plan source with no plan: every `plan_tasks:` stage waits for the planner.
pub(crate) struct NoPlan;

impl PlanSource for NoPlan {
    fn load(&self, _path: &Path) -> anyhow::Result<Option<Plan>> {
        Ok(None)
    }
}
