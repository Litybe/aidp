use crate::models::port::PlanSource;
use crate::plan::Plan;
use std::path::Path;

/// Serves the same fixture plan for every `plan_tasks:` stage, or none at all.
pub(crate) struct FixturePlanSource {
    pub(crate) plan: Option<Plan>,
}

impl PlanSource for FixturePlanSource {
    fn load(&self, _path: &Path) -> anyhow::Result<Option<Plan>> {
        Ok(self.plan.clone())
    }
}
