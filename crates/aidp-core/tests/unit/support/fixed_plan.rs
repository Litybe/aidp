use crate::models::port::PlanSource;
use crate::plan::{Plan, Task, KINDS, PLAN_SCHEMA_ID};
use std::path::Path;

const SUMMARY: &str = "s";
const TASK_TITLE: &str = "t";
const TASK_PROMPT: &str = "p";
const SINGLE_TASK_ID: &str = "T1";
const SINGLE_TASK_KIND: &str = "code";
const SINGLE_TASK_FILE: &str = "src/x.rs";
const KIND_TASK_ID_PREFIX: &str = "T-";

/// A plan source serving the same plan for every `plan_tasks:` stage.
pub(crate) struct FixedPlan(pub(crate) Plan);

impl FixedPlan {
    fn task(id: String, kind: &str, file: &str) -> Task {
        Task {
            id,
            title: TASK_TITLE.to_string(),
            kind: kind.to_string(),
            prompt: TASK_PROMPT.to_string(),
            files: vec![file.to_string()],
            ..Task::default()
        }
    }

    fn plan_of(tasks: Vec<Task>) -> Self {
        Self(Plan {
            schema: Some(PLAN_SCHEMA_ID.to_string()),
            summary: SUMMARY.to_string(),
            tasks,
            ..Plan::default()
        })
    }

    /// A valid plan holding one `code` task.
    pub(crate) fn one_task() -> Self {
        Self::plan_of(vec![Self::task(
            SINGLE_TASK_ID.to_string(),
            SINGLE_TASK_KIND,
            SINGLE_TASK_FILE,
        )])
    }

    /// A valid plan holding one task per task kind.
    pub(crate) fn one_task_per_kind() -> Self {
        Self::plan_of(
            KINDS
                .iter()
                .map(|kind| {
                    Self::task(
                        format!("{KIND_TASK_ID_PREFIX}{kind}"),
                        kind,
                        &format!("src/{kind}.rs"),
                    )
                })
                .collect(),
        )
    }
}

impl PlanSource for FixedPlan {
    fn load(&self, _path: &Path) -> anyhow::Result<Option<Plan>> {
        Ok(Some(self.0.clone()))
    }
}
