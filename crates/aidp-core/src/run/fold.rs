//! Pure fold of a Workflow result object into a run state.

use super::{RunState, StageStatus};
use crate::models::constants::skills::{PLAN_TASK_SEPARATOR, SIGNALS_KEY, SKILLS_NOT_LOADED_KEY};
use crate::models::constants::workflow_result::{
    DURATION_MS_KEY, FOLLOW_UPS_KEY, GUARD_FINDINGS_KEY, METRICS_KEY, OUTPUTS_KEY, RESULTS_KEY,
    STATUS_KEY, SUMMARY_KEY, TOKENS_USED_KEY,
};
use crate::models::enums::envelope_status::EnvelopeStatus;
use crate::models::enums::workflow_status::WorkflowStatus;
use crate::plan::TaskStatus;
use std::path::PathBuf;

type ResultMap = serde_json::Map<String, serde_json::Value>;

/// Returns `state` with `result` folded in: workflow status, guard findings, per-stage and
/// plan-task outcomes. It reads no disk, no clock and no process.
pub fn fold_workflow_result(
    state: &RunState,
    result: &serde_json::Value,
    loop_containers: &[String],
) -> RunState {
    let mut folded = state.clone();
    if let Some(status) = result.get(STATUS_KEY).and_then(|v| v.as_str()) {
        folded.last_workflow_status = Some(status.to_string());
    }
    let finished = WorkflowStatus::from_result(result) == Some(WorkflowStatus::Done);
    let results = result.get(RESULTS_KEY).and_then(|v| v.as_object());

    if let Some(guard_findings) = result.get(GUARD_FINDINGS_KEY).and_then(|v| v.as_array()) {
        folded.guard_findings = guard_findings.clone();
    }

    fold_stage_outcomes(&mut folded, results, loop_containers, finished);
    fold_plan_task_outcomes(&mut folded, results, finished);
    folded
}

/// Envelope status of a reply, `Completed` when the reply names none.
fn envelope_status(env: &serde_json::Value) -> EnvelopeStatus {
    env.get(STATUS_KEY)
        .and_then(|v| v.as_str())
        .map_or(EnvelopeStatus::Completed, EnvelopeStatus::parse)
}

/// Applies each stage's envelope from `results`, and settles the stages that have none once the
/// run is finished.
fn fold_stage_outcomes(
    state: &mut RunState,
    results: Option<&ResultMap>,
    loop_containers: &[String],
    finished: bool,
) {
    for stage in state.stages.iter_mut() {
        if let Some(env) = results.and_then(|m| m.get(&stage.name)) {
            stage.status = if envelope_status(env).is_success() {
                StageStatus::Success
            } else {
                StageStatus::Cancel
            };
            if let Some(metrics) = env.get(METRICS_KEY) {
                if let Some(t) = metrics.get(TOKENS_USED_KEY).and_then(|v| v.as_u64()) {
                    stage.tokens_used = Some(t);
                }
                if let Some(d) = metrics.get(DURATION_MS_KEY).and_then(|v| v.as_u64()) {
                    stage.duration_ms = Some(d);
                }
            }
            if let Some(outs) = env.get(OUTPUTS_KEY).and_then(|v| v.as_object()) {
                for (k, v) in outs {
                    if let Some(p) = v.as_str() {
                        stage.outputs.insert(k.clone(), PathBuf::from(p));
                    }
                }
            }
            if let Some(s) = env.get(SUMMARY_KEY).and_then(|v| v.as_str()) {
                let s = s.trim();
                if !s.is_empty() {
                    stage.summary = Some(s.to_string());
                }
            }
            if let Some(f) = env.get(FOLLOW_UPS_KEY).and_then(|v| v.as_array()) {
                let items: Vec<String> = f
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !items.is_empty() {
                    stage.follow_ups = items;
                }
            }
            stage.skills_not_loaded = missing_skills(env, results, &stage.name);
        } else if loop_containers.contains(&stage.name) {
            if finished && stage.status == StageStatus::Pending {
                stage.status = StageStatus::Success;
            }
        } else if finished && stage.status == StageStatus::Pending {
            stage.status = StageStatus::Skipped;
        }
    }
}

/// Applies each plan task's envelope (`<stage>#<task_id>`) from `results`.
fn fold_plan_task_outcomes(state: &mut RunState, results: Option<&ResultMap>, finished: bool) {
    for sched in state.plan_schedules.values_mut() {
        for t in sched.tasks.iter_mut() {
            let key = format!("{}#{}", sched.stage, t.id);
            let Some(env) = results.and_then(|m| m.get(&key)) else {
                if finished && t.status == TaskStatus::Pending {
                    t.status = TaskStatus::Skipped;
                }
                continue;
            };
            let status = envelope_status(env);
            t.status = if status.is_success() {
                TaskStatus::Success
            } else {
                TaskStatus::from(
                    env.get(STATUS_KEY)
                        .and_then(|v| v.as_str())
                        .unwrap_or(status.as_str()),
                )
            };
            if let Some(n) = env
                .get(METRICS_KEY)
                .and_then(|m| m.get(TOKENS_USED_KEY))
                .and_then(|v| v.as_u64())
            {
                t.tokens_used = Some(n);
            }
        }
    }
}

/// True when the stage succeeded yet none of its plan tasks has a recorded outcome.
pub(super) fn plan_task_accounting_is_inconsistent(
    state: &RunState,
    sched: &crate::plan::ScheduleState,
) -> bool {
    if sched.tasks.is_empty() {
        return false;
    }
    let stage_succeeded = state
        .stages
        .iter()
        .any(|s| s.name == sched.stage && s.status == StageStatus::Success);
    stage_succeeded && !sched.tasks.iter().any(|t| t.status == TaskStatus::Success)
}

/// Every plan-driven stage whose task accounting contradicts its own outcome.
pub fn plan_accounting_anomalies(state: &RunState) -> Vec<String> {
    state
        .plan_schedules
        .values()
        .filter(|sched| plan_task_accounting_is_inconsistent(state, sched))
        .map(|sched| {
            format!(
                "plan `{stage}`: stage `{stage}` succeeded, yet 0/{n} of its task(s) recorded an outcome \
                 — the Workflow result carried no `{stage}#<task_id>` key. This is a reporting failure, \
                 not a work failure: read what the run actually produced before re-running anything. \
                 Likeliest causes — the result was piped in still wrapped in its enclosing envelope \
                 (`reconcile` wants the workflow object itself, i.e. `.result`), or the emitted script \
                 did not record per-task results for this stage.",
                stage = sched.stage,
                n = sched.tasks.len(),
            )
        })
        .collect()
}

/// Reads the `signals.skills_not_loaded` list of one reply envelope.
fn skills_from_envelope(env: &serde_json::Value) -> Vec<String> {
    env.get(SIGNALS_KEY)
        .and_then(|v| v.get(SKILLS_NOT_LOADED_KEY))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Skills a stage failed to load: its own, then each plan task's as `<task>: <skill>`.
/// Deduplicated, in result order.
fn missing_skills(
    env: &serde_json::Value,
    results: Option<&ResultMap>,
    stage_name: &str,
) -> Vec<String> {
    let mut missing = skills_from_envelope(env);
    let prefix = format!("{stage_name}{PLAN_TASK_SEPARATOR}");
    for (key, task_env) in results.into_iter().flatten() {
        let Some(task_id) = key.strip_prefix(&prefix) else {
            continue;
        };
        for skill in skills_from_envelope(task_env) {
            let entry = format!("{task_id}: {skill}");
            if !missing.contains(&entry) {
                missing.push(entry);
            }
        }
    }
    missing
}

#[cfg(test)]
#[path = "../../tests/unit/run_fold_tests.rs"]
mod tests;
