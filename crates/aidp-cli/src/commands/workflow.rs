use super::run_loader::load_run;
use super::CommandOutput;
use crate::args::WorkflowOp;
use crate::ctx::Ctx;
use crate::models::constants::{
    AWAITING_COMPOSITE_NOTE, AWAITING_PLAN_NOTE, PHASE_AWAITING_COMPOSITE, PHASE_AWAITING_PLAN,
    PHASE_PLAN_COMPILED,
};
use crate::models::plan_status_envelope::PlanStatusEnvelope;
use crate::models::plan_task_status_count::PlanTaskStatusCount;
use crate::models::workflow_phase::{
    AwaitingCompositePhase, AwaitingPlanPhase, PlanCompiledPhase, PlanSummary,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const WORKFLOW_SCRIPT_FILE: &str = "workflow.js";
const PLAN_SCHEDULE_FILE: &str = "plan-schedule.json";

pub fn run(ctx: &Ctx, op: WorkflowOp) -> anyhow::Result<CommandOutput> {
    match op {
        WorkflowOp::Emit { run, out } => emit(ctx, &run, out),
        WorkflowOp::PlanStatus { run } => plan_status(ctx, &run, ctx.format()),
    }
}

fn emit(ctx: &Ctx, run: &str, out: Option<PathBuf>) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let mut run_state = load_run(ctx, &workspace, run)?;
    let (path, emitted) = emit_to_disk(ctx, &workspace, &mut run_state, out)?;
    let phase = workflow_phase_line(&emitted)?;
    Ok(CommandOutput::text(path.display().to_string())
        .with_diagnostics(phase.into_iter().collect()))
}

fn plan_status(
    ctx: &Ctx,
    run: &str,
    format: Option<aidp_core::Format>,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let run_state = load_run(ctx, &workspace, run)?;
    let emitted = aidp_core::workflow_emit::compile(&run_state)?;
    let mut schedules = emitted.plan_schedules;
    for sched in schedules.iter_mut() {
        if let Some(prev) = run_state.plan_schedules.get(&sched.stage) {
            aidp_core::run::carry_over_task_outcomes(prev, sched);
        }
    }
    let task_status_counts = schedules
        .iter()
        .map(|s| PlanTaskStatusCount {
            counts: tally_task_statuses(s.tasks.iter()),
            stage: &s.stage,
        })
        .collect();
    let envelope = PlanStatusEnvelope {
        awaiting_plan: &emitted.awaiting_plan,
        compiled: emitted.awaiting_plan.is_none(),
        pipeline_id: &run_state.pipeline_id,
        plan_schedules: &schedules,
        run_id: &run_state.run_id,
        task_status_counts,
    };
    let format = format.unwrap_or_default();
    let value = if format == aidp_core::Format::Ndjson {
        serde_json::to_value(&schedules)?
    } else {
        serde_json::to_value(&envelope)?
    };
    let rendered = crate::output::render_value(format, &value)?;
    Ok(CommandOutput::text(rendered))
}

/// Tallies plan-task outcomes by raw `TaskState::status`, with `cancelled` under its own key.
/// Kept apart from `failed` because a never-dispatched task is not an errored one.
pub fn tally_task_statuses<'a>(
    tasks: impl Iterator<Item = &'a aidp_core::plan::TaskState>,
) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for task in tasks {
        *counts.entry(task.status.to_string()).or_insert(0) += 1;
    }
    counts
}

/// `<vault>/Pipelines/<repo>/.state/run-in-flight` — "a v7 run is underway in
/// this repo".
fn run_in_flight_path(workspace_root: &Path) -> PathBuf {
    let vault = aidp_core::vault::resolve(None, workspace_root);
    aidp_core::vault::repo_state_path_in(
        vault.as_ref(),
        workspace_root,
        aidp_core::vault::RUN_IN_FLIGHT_FILE,
    )
}

/// The run id recorded in the in-flight marker, for callers with no id of their own.
/// Used by the Stop hook, which never knows which run the session just drove.
pub fn in_flight_run_id(workspace_root: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(run_in_flight_path(workspace_root)).ok()?;
    let run_json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    run_json
        .get("run_id")?
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn mark_run_in_flight(ctx: &Ctx, workspace_root: &Path, run_id: &str, run_dir: &Path) {
    let marker = run_in_flight_path(workspace_root);
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::json!({
        "run_id": run_id,
        "run_dir": run_dir.to_string_lossy(),
        "at": ctx.clock.now_rfc3339(),
    });
    let _ = std::fs::write(&marker, body.to_string());
}

pub fn clear_run_in_flight(workspace_root: &Path) {
    let _ = std::fs::remove_file(run_in_flight_path(workspace_root));
}

/// The compilation phase as one JSON line, for stderr so stdout stays the script path.
/// A pass resolving plan tasks reports the schedule and not a later composite stop.
fn workflow_phase_line(
    emitted: &aidp_core::workflow_emit::Emitted,
) -> anyhow::Result<Option<String>> {
    let line = if let Some(awaiting) = &emitted.awaiting_plan {
        serde_json::to_string(&AwaitingPlanPhase {
            note: AWAITING_PLAN_NOTE,
            phase: PHASE_AWAITING_PLAN,
            plan_path: &awaiting.plan_path,
            producer: &awaiting.producer,
            stage: &awaiting.stage,
            warnings: &emitted.warnings,
        })?
    } else if !emitted.plan_schedules.is_empty() {
        let plans = emitted
            .plan_schedules
            .iter()
            .map(|schedule| PlanSummary {
                max_parallel: schedule.max_parallel,
                serializations: schedule.serializations.len(),
                stage: &schedule.stage,
                tasks: schedule.tasks.len(),
                warnings: schedule.warnings.len(),
                waves: schedule.waves.len(),
            })
            .collect();
        serde_json::to_string(&PlanCompiledPhase {
            phase: PHASE_PLAN_COMPILED,
            plans,
            warnings: &emitted.warnings,
        })?
    } else if let Some(awaiting) = &emitted.awaiting_composite {
        serde_json::to_string(&AwaitingCompositePhase {
            composite: &awaiting.composite,
            note: AWAITING_COMPOSITE_NOTE,
            phase: PHASE_AWAITING_COMPOSITE,
            stage: &awaiting.stage,
            warnings: &emitted.warnings,
        })?
    } else {
        return Ok(None);
    };
    Ok(Some(line))
}

/// Compile `state` into `workflow.js` and write it to disk, the only writer of that file.
/// The knowledge-base read is frozen on the run's first emit, so a re-emit reuses it.
pub fn emit_to_disk(
    ctx: &Ctx,
    workspace: &Path,
    run_state: &mut aidp_core::run::RunState,
    out: Option<PathBuf>,
) -> anyhow::Result<(PathBuf, aidp_core::workflow_emit::Emitted)> {
    aidp_core::run::ensure_knowledge_snapshot(run_state)?;
    let emitted = aidp_core::workflow_emit::compile(run_state)?;
    let path = out.unwrap_or_else(|| run_state.run_dir.join(WORKFLOW_SCRIPT_FILE));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&path, &emitted.script)
        .map_err(|e| anyhow::anyhow!("write {}: {e}", path.display()))?;
    mark_run_in_flight(ctx, workspace, &run_state.run_id, &run_state.run_dir);
    if !emitted.plan_schedules.is_empty() {
        persist_plan_schedules(run_state, &emitted)?;
    }
    Ok((path, emitted))
}

/// Persists how the plan was parallelised so `plan-status`, `summary` and
/// `reconcile` can report on the tasks, and the decision is auditable.
fn persist_plan_schedules(
    run_state: &mut aidp_core::run::RunState,
    emitted: &aidp_core::workflow_emit::Emitted,
) -> anyhow::Result<()> {
    aidp_core::run::record_plan_schedules(run_state, emitted.plan_schedules.clone())?;
    let schedule_path = run_state.run_dir.join(PLAN_SCHEDULE_FILE);
    let _ = std::fs::write(
        &schedule_path,
        serde_json::to_vec_pretty(&emitted.plan_schedules)?,
    );
    Ok(())
}
