//! Folding a Workflow result into the persisted run state.

use super::cost::attach_cost;
use super::fold::{fold_workflow_result, plan_accounting_anomalies};
use super::layout::save;
use super::RunState;
use crate::clock::SystemClock;
use crate::cost::PriceTable;
use crate::git::GitCli;
use crate::id::SystemIdGen;
use crate::models::enums::workflow_status::WorkflowStatus;
use crate::models::run_state::RunPorts;
use crate::pipeline;
use crate::plan::TaskStatus;

/// Persist the plan schedules resolved by `aidp workflow emit` (pass 2 of a `plan_tasks:`
/// stage).
pub fn record_plan_schedules(
    state: &mut RunState,
    schedules: Vec<crate::plan::ScheduleState>,
) -> anyhow::Result<()> {
    for mut fresh in schedules {
        if let Some(prev) = state.plan_schedules.get(&fresh.stage) {
            carry_over_task_outcomes(prev, &mut fresh);
        }
        state.plan_schedules.insert(fresh.stage.clone(), fresh);
    }
    save(state)?;
    Ok(())
}

/// Copy the outcomes already recorded in `prev` onto the freshly compiled `fresh` schedule,
/// task by task (matched on id).
pub fn carry_over_task_outcomes(
    prev: &crate::plan::ScheduleState,
    fresh: &mut crate::plan::ScheduleState,
) {
    for t in fresh.tasks.iter_mut() {
        if let Some(old) = prev.tasks.iter().find(|o| o.id == t.id) {
            if old.status != TaskStatus::Pending {
                t.status = old.status.clone();
                t.tokens_used = old.tokens_used;
            }
        }
    }
}

/// Fold a Workflow result object back into run-state so `aidp run summary` / `status` reflect
/// a completed v7 run.
#[tracing::instrument(skip_all, fields(run_id = %state.run_id))]
pub fn reconcile(state: &mut RunState, result: &serde_json::Value) -> anyhow::Result<Vec<String>> {
    let clock = SystemClock;
    let ids = SystemIdGen;
    let git = GitCli;
    let prices = PriceTable::from_env_or_embedded();
    let ports = RunPorts {
        clock: &clock,
        ids: &ids,
        git: &git,
        prices: &prices,
    };
    reconcile_with(state, result, &ports)
}

/// [`reconcile`] with its clock/git/pricing ports injected — see [`RunPorts`].
pub fn reconcile_with(
    state: &mut RunState,
    result: &serde_json::Value,
    ports: &RunPorts,
) -> anyhow::Result<Vec<String>> {
    let finished = WorkflowStatus::from_result(result) == Some(WorkflowStatus::Done);

    let pipe = pipeline::load(&state.pipeline_path).ok();

    let loop_containers: Vec<String> = pipe
        .as_ref()
        .map(|p| {
            p.stages
                .iter()
                .filter(|s| s.is_loop())
                .map(|s| s.name.clone())
                .collect()
        })
        .unwrap_or_default();

    *state = fold_workflow_result(state, result, &loop_containers);

    for stage in state.stages.iter_mut() {
        if stage.outputs.is_empty() {
            continue;
        }
        stage.verification = crate::verify::verify_outputs(stage)
            .into_iter()
            .map(|v| format!("{}: {}", v.path.display(), v.signals.join(", ")))
            .collect();
    }

    if let Some(base_sha) = &state.base_sha {
        let paths = ports.git.changed(&state.workspace_root, base_sha);
        state.vault_leaks = crate::vault_leak::detect(&paths, state.vault_root.is_some());
    }

    if finished {
        const COMMIT_LOG_WINDOW: usize = 50;
        if let Some(head) = ports.git.head(&state.workspace_root) {
            state.head_sha_end = Some(head);
        }
        if let Ok(commits) = ports
            .git
            .log(&state.workspace_root, COMMIT_LOG_WINDOW, None)
        {
            state.commits = commits.into_iter().map(|c| c.sha).collect();
        }

        let verify_scope = pipe
            .as_ref()
            .map(|p| p.settings.verify_scope)
            .unwrap_or(true);
        if verify_scope {
            if let Ok(violations) = crate::scope::verify_run(state) {
                state.scope_violations = violations;
            }
        }
    }

    attach_cost(state, ports);

    save(state)?;
    Ok(plan_accounting_anomalies(state))
}
