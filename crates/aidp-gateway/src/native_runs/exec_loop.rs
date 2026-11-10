//! Drives the execution plan of a run, one session per stage.
//! Mirrors the Workflow semantics: conditions, loops, fan-out, gates, composites, plan tasks.

use super::autonomy::{auto_answers, blocked_journal, needs_auto_answer, retry_note};
use super::dispatch::Dispatcher;
use super::reply::{HAS_BLOCKING_KEY, REPLY_FILE, UNEVALUATED_KEY};
use super::scope_guard;
use super::state::{RunControl, RunStore};
use super::task_isolation::{self, IsolatedTask};
use super::{
    NativeDeps, EVENT_RUN_ENDED, EVENT_RUN_STAGE_CHANGED, EVENT_RUN_STARTED, EVENT_RUN_WAITING_GATE,
};
use crate::models::constants::plan_task_prompt::SERIALIZED_HEADING;
use crate::models::enums::apply_verdict::ApplyVerdict;
use crate::models::enums::native_reason::NativeReason;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::native_stage_status::NativeStageStatus;
use crate::models::enums::session_outcome::SessionOutcome;
use crate::models::native_reply_envelope::NativeReplyEnvelope;
use crate::models::native_run_event::NativeRunEvent;
use crate::models::plan_slots::PlanSlots;
use crate::models::session_job::SessionJob;
use crate::models::session_run::SessionRun;
use crate::models::task_outcome::TaskOutcome;
use crate::models::task_worktree::TaskWorktree;
use crate::models::task_write_baseline::TaskWriteBaseline;
use aidp_core::exec_plan::{
    compile_exec_plan, ContractFlag, FanoutItems, PassEnd, SkipReason, StageSpec, Step,
};
use aidp_core::expr;
use aidp_core::findings::{self, MergeInput};
use aidp_core::models::enums::{AgentRole, ItemFailurePolicy};
use aidp_core::pipeline::{self, Pipeline, Settings};
use aidp_core::plan::{self, Schedule, ScheduledTask, Serialization, TaskStatus};
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner};
use aidp_core::run::EnvelopeStatus;
use aidp_core::workflow_emit::DiskPlanSource;
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};

const PROCESS_RUNNER: OsProcessRunner = OsProcessRunner;
const MAX_PARALLEL_SETTING: &str = "max_parallel";
const DEFAULT_MAX_PARALLEL: usize = 3;
const GATE_POLL: Duration = Duration::from_millis(500);
const MAX_COMPILE_PASSES: usize = 64;
const WILDCARD_GATE: &str = "*";
const ROW_SEPARATOR: char = '#';
const TASKS_DIR: &str = "tasks";
const SHARDS_DIR: &str = "shards";
const SHARD_PREFIX: &str = "shard-";
const FANOUT_REF_PREFIX: &str = "stages.";
const FANOUT_SIGNALS_MARKER: &str = ".signals.";
const FINDINGS_OUTPUT: &str = "findings";
const LABEL_SEPARATOR: &str = ", ";
const REPLY_OVERRIDE_HEADING: &str = "\n\n## Reply location\n\n";
const SHARD_HEADING: &str = "\n\n## Fan-out shard\n\n";
const TASK_HEADING: &str = "\n\n## Plan task\n\n";
const MERGE_HEADING: &str = "\n\n## Merge\n\n";

/// True when a reply with `status` stops the run: a hard failure always, `needs_review` unless soft.
pub fn pauses_run(status: EnvelopeStatus, soft: bool) -> bool {
    match status {
        EnvelopeStatus::Completed => false,
        EnvelopeStatus::NeedsReview => !soft,
        _ => true,
    }
}

/// True while a correction loop must run its body again.
pub fn loop_continues(condition_holds: bool, iteration: u32, max: u32) -> bool {
    condition_holds && iteration < max
}

/// True when `auto` lets `stage` through its gate without asking.
pub fn auto_approves(auto: &[String], stage: &str) -> bool {
    auto.iter()
        .any(|gate| gate == WILDCARD_GATE || gate == stage)
}

/// True when the run must stop and wait for a human before `stage`.
pub fn gate_blocks(gated: bool, approved: bool, stage: &str, auto: &[String]) -> bool {
    gated && !approved && !auto_approves(auto, stage)
}

/// The reply of a finished session when it still waits for a human.
fn blocked_reply(run: &SessionRun) -> Option<&NativeReplyEnvelope> {
    let SessionOutcome::Completed(envelope) = &run.outcome else {
        return None;
    };
    let status = EnvelopeStatus::parse(&envelope.status);
    let open = status != EnvelopeStatus::Completed
        && needs_auto_answer(status, &envelope.ask_user_questions);
    open.then_some(envelope)
}

/// Whether a signal value counts as set: `true`, a positive number or a non-empty string.
pub fn signal_is_set(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|count| count > 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        _ => false,
    }
}

/// The reason a stage is skipped when its runtime condition is false.
pub fn when_skip_reason(expression: &str) -> String {
    format!("when `{expression}` is false")
}

/// Concurrency cap of parallel groups: `settings.max_parallel`, at least one, default three.
pub fn max_parallel(settings: &Settings) -> usize {
    settings
        .extra
        .get(MAX_PARALLEL_SETTING)
        .and_then(|value| value.as_u64())
        .map(|count| (count as usize).max(1))
        .unwrap_or(DEFAULT_MAX_PARALLEL)
}

/// Splits fan-out items into shards: at most `max` of them, the overflow folded into the last
/// shard when `fold` holds and dropped otherwise.
pub fn shard_labels(items: Vec<String>, max: u32, fold: bool) -> Vec<Vec<String>> {
    let cap = (max as usize).max(1);
    if items.len() <= cap {
        return items.into_iter().map(|item| vec![item]).collect();
    }
    let mut shards: Vec<Vec<String>> = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        if index < cap {
            shards.push(vec![item]);
        } else if fold {
            shards[cap - 1].push(item);
        }
    }
    shards
}

/// The `<stage>#<id>` row name of a plan task, shard or other sub-session.
pub fn row_name(stage: &str, id: &str) -> String {
    format!("{stage}{ROW_SEPARATOR}{id}")
}

fn skip_reason_text(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::StaticWhen => NativeReason::StaticWhenFalse.as_str(),
        SkipReason::CompositeWhen => NativeReason::CompositeWhenFalse.as_str(),
    }
}

fn value_label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Every stage name in `steps`, nested ones included.
fn step_names(steps: &[Step]) -> Vec<String> {
    let mut names = Vec::new();
    for step in steps {
        match step {
            Step::Stage(spec) => names.push(spec.name.clone()),
            Step::Parallel { stages } => names.extend(stages.iter().map(|spec| spec.name.clone())),
            Step::Loop {
                container, body, ..
            } => {
                names.push(container.clone());
                names.extend(step_names(body));
            }
            Step::Fanout { stage, .. } | Step::PlanTasks { stage, .. } => {
                names.push(stage.name.clone())
            }
            Step::Skipped { stage, .. } => names.push(stage.clone()),
            Step::CompositeBoundary { .. } => {}
        }
    }
    names
}

fn is_executor(spec: &StageSpec) -> bool {
    spec.agent == AgentRole::Executor
}

fn bullet_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {item}\n"))
        .collect::<String>()
}

fn reply_override(reply_dir: &Path) -> String {
    format!(
        "{REPLY_OVERRIDE_HEADING}For this session only, write the reply envelope to `{}` instead of the path above.",
        reply_dir.join(REPLY_FILE).display()
    )
}

fn task_section(
    scheduled: &ScheduledTask,
    shared_files: &[String],
    serializations: &[Serialization],
    alone: bool,
) -> String {
    let task = &scheduled.task;
    let mut text = format!(
        "{TASK_HEADING}`{}` — {}\n\nkind: `{}`\n\n",
        task.id, task.title, task.kind
    );
    text.push_str(&format!("{}\n\n", task.prompt));
    if !task.instructions.is_empty() {
        text.push_str("### Steps\n\n");
        text.push_str(&bullet_list(&task.instructions));
    }
    text.push_str("\n### Files you own (your write set)\n\n");
    text.push_str(&bullet_list(&task.files));
    if !task.reads.is_empty() {
        text.push_str("\n### Read for context\n\n");
        text.push_str(&bullet_list(&task.reads));
    }
    if !task.verification.is_empty() {
        text.push_str(&format!("\n### Verification\n\n{}\n", task.verification));
    }
    if !task.acceptance.is_empty() {
        text.push_str("\n### Acceptance criteria\n\n");
        text.push_str(&bullet_list(&task.acceptance));
    }
    text.push_str(&serialization_lines(&task.id, serializations));
    if !shared_files.is_empty() {
        text.push_str("\n### Hands off (shared files)\n\n");
        text.push_str(&bullet_list(shared_files));
        text.push_str("Describe the change you need in these files in `follow_ups`; the merge session applies them.\n");
    }
    if !alone {
        text.push_str(
            "\nDo only this task. A merge session runs afterwards and owns the stage outputs.\n",
        );
    }
    text
}

/// Lines naming the tasks ordered with `task_id` because both write the same files.
fn serialization_lines(task_id: &str, serializations: &[Serialization]) -> String {
    let concerning: Vec<String> = serializations
        .iter()
        .filter(|edge| edge.before == task_id || edge.after == task_id)
        .map(|edge| {
            format!(
                "`{}` runs before `{}`: both write {}",
                edge.before,
                edge.after,
                edge.files.join(LABEL_SEPARATOR)
            )
        })
        .collect();
    if concerning.is_empty() {
        return String::new();
    }
    format!("{SERIALIZED_HEADING}{}", bullet_list(&concerning))
}

fn task_label(spec: &StageSpec, task_id: &str, alone: bool) -> String {
    if alone {
        spec.name.clone()
    } else {
        row_name(&spec.name, task_id)
    }
}

/// Maps the status a task session ended with to a task outcome.
fn outcome_of(status: NativeRunStatus, tokens: Option<u64>) -> TaskOutcome {
    let outcome = match status {
        NativeRunStatus::Running => TaskOutcome::succeeded(tokens, Vec::new()),
        NativeRunStatus::Cancelled => TaskOutcome::cancelled(),
        _ => TaskOutcome {
            reason: None,
            ..TaskOutcome::failed(NativeReason::TaskFailed)
        },
    };
    TaskOutcome {
        run_status: status,
        tokens,
        ..outcome
    }
}

/// Concurrency cap of a plan stage: the stage's `max`, bounded by `settings.max_parallel`.
pub fn plan_cap(stage_max: u32, settings_cap: usize) -> usize {
    (stage_max.max(1) as usize).min(settings_cap.max(1))
}

/// Runs `tasks` on at most `slots.cap()` threads, each started once its dependencies succeeded.
/// A panicking or erroring worker becomes a failed outcome; the first error comes back beside them.
pub fn drive_tasks(
    slots: &PlanSlots,
    tasks: &[ScheduledTask],
    cancelled: &(dyn Fn() -> bool + Sync),
    halt_on_failure: bool,
    worker: &(dyn Fn(&ScheduledTask) -> anyhow::Result<TaskOutcome> + Sync),
) -> (Vec<(usize, TaskOutcome)>, Option<anyhow::Error>) {
    let halted = AtomicBool::new(false);
    let outcomes: Mutex<Vec<(usize, TaskOutcome)>> = Mutex::new(Vec::new());
    let first_error: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    let may_launch = || !halted.load(Ordering::SeqCst) && !cancelled();
    std::thread::scope(|scope| {
        while let Some(index) = slots.acquire(tasks, &may_launch) {
            let (halted, outcomes, first_error) = (&halted, &outcomes, &first_error);
            scope.spawn(move || {
                let scheduled = &tasks[index];
                let outcome = match scope.spawn(|| worker(scheduled)).join() {
                    Ok(Ok(outcome)) => outcome,
                    Ok(Err(error)) => {
                        first_error
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .get_or_insert(error);
                        TaskOutcome::failed(NativeReason::TaskFailed)
                    }
                    Err(_) => TaskOutcome::failed(NativeReason::TaskPanicked),
                };
                if !outcome.is_success() && halt_on_failure {
                    halted.store(true, Ordering::SeqCst);
                }
                slots.release(&scheduled.task.id, outcome.status.clone());
                outcomes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push((index, outcome));
            });
        }
    });
    let mut collected = outcomes
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    collected.sort_by_key(|(index, _)| *index);
    let error = first_error
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    (collected, error)
}

/// The text persisted on a failed task row: the reason, then the offending paths when any.
fn row_reason(reason: NativeReason, violations: &[String]) -> String {
    if violations.is_empty() {
        return reason.as_str().to_string();
    }
    format!("{reason}: {}", violations.join(LABEL_SEPARATOR))
}

/// Fails both tasks of every pair whose run windows overlapped and that wrote the same paths.
/// Returns the paths to restore once every sibling has finished.
pub fn flag_overlaps(
    slots: &PlanSlots,
    tasks: &[ScheduledTask],
    outcomes: &mut [(usize, TaskOutcome)],
) -> Vec<String> {
    let mut conflicts: Vec<(usize, Vec<String>)> = Vec::new();
    for (position, (first_index, first)) in outcomes.iter().enumerate() {
        let peers = slots.overlapping_peers(&tasks[*first_index].task.id);
        for (second_index, second) in outcomes.iter().skip(position + 1) {
            if !peers.contains(&tasks[*second_index].task.id) {
                continue;
            }
            let shared = scope_guard::overlapping_writes(&first.written, &second.written);
            if shared.is_empty() {
                continue;
            }
            conflicts.push((*first_index, shared.clone()));
            conflicts.push((*second_index, shared));
        }
    }
    let mut restore: Vec<String> = Vec::new();
    for (index, paths) in conflicts {
        let Some((_, outcome)) = outcomes.iter_mut().find(|(found, _)| *found == index) else {
            continue;
        };
        outcome.status = TaskStatus::Failed;
        outcome.run_status = NativeRunStatus::Paused;
        outcome.reason = Some(NativeReason::OverlappingWrites);
        for path in paths {
            if !outcome.violations.contains(&path) {
                outcome.violations.push(path.clone());
            }
            if !restore.contains(&path) {
                restore.push(path);
            }
        }
    }
    restore
}

/// Files of the tasks running at the same time that a task may write without failing.
/// An isolated task has no peer files in its tree, so none are allowed.
fn peer_files(
    scheduled: &ScheduledTask,
    schedule: &Schedule,
    slots: &PlanSlots,
    isolated: bool,
) -> Vec<String> {
    if isolated {
        return Vec::new();
    }
    let peers = slots.overlapping_peers(&scheduled.task.id);
    schedule
        .tasks
        .iter()
        .filter(|other| peers.contains(&other.task.id))
        .flat_map(|other| other.task.files.iter().cloned())
        .collect()
}

/// Checks what the task wrote in `workspace` against its files, restoring stray writes on a
/// violation.
fn verify_task_scope(
    workspace: &Path,
    scheduled: &ScheduledTask,
    schedule: &Schedule,
    slots: &PlanSlots,
    baseline: &TaskWriteBaseline,
    isolated: bool,
    outcome: &mut TaskOutcome,
) -> anyhow::Result<()> {
    let mut written = scope_guard::changed_since(workspace, baseline, &PROCESS_RUNNER);
    let peer_files = peer_files(scheduled, schedule, slots, isolated);
    let verdict = scope_guard::verify_task_writes(
        &written,
        &scheduled.task.files,
        &peer_files,
        &schedule.shared_files,
    );
    if verdict.is_clean() {
        let peer_owned: Vec<String> = peer_files
            .iter()
            .filter(|path| !scheduled.task.files.contains(path))
            .map(|path| plan::normalize_plan_path(path))
            .collect();
        written.retain(|path| !peer_owned.contains(&plan::normalize_plan_path(path)));
        outcome.written = written;
        return Ok(());
    }
    scope_guard::restore_task_violations(
        workspace,
        &verdict.violations,
        baseline,
        &PROCESS_RUNNER,
    )?;
    written.retain(|path| !verdict.violations.contains(path));
    outcome.written = written;
    // The restore above dropped them from the isolated tree, so they never reach the ticket worktree.
    if isolated {
        warn!(task = %scheduled.task.id, paths = ?verdict.violations, "dropped isolated writes outside the task's files");
        return Ok(());
    }
    outcome.status = TaskStatus::Failed;
    outcome.run_status = NativeRunStatus::Paused;
    outcome.reason = Some(NativeReason::WroteOutsideFiles);
    outcome.violations = verdict.violations;
    Ok(())
}

/// Applies the isolated task's changes onto the ticket worktree; a conflict fails the task.
fn apply_task(
    ticket: &Path,
    tree: &TaskWorktree,
    outcome: &mut TaskOutcome,
    runner: &dyn ProcessRunner,
) {
    match task_isolation::apply_back(ticket, tree, &outcome.written, runner) {
        Ok(ApplyVerdict::Applied) => {}
        Ok(ApplyVerdict::Conflict(paths)) => {
            *outcome = TaskOutcome {
                violations: paths,
                ..TaskOutcome::failed(NativeReason::ApplyConflict)
            };
        }
        Err(error) => {
            warn!(%error, "could not apply the isolated task back");
            *outcome = TaskOutcome::failed(NativeReason::TaskFailed);
        }
    }
}

/// True when the session ended with a reply that does not stop the run.
fn session_succeeded(run: &SessionRun) -> bool {
    matches!(
        &run.outcome,
        SessionOutcome::Completed(envelope)
            if !pauses_run(EnvelopeStatus::parse(&envelope.status), true)
    )
}

fn lock_ticket(lock: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a plan stage needs to run its tasks in isolated worktrees.
struct IsolationSetup<'a> {
    root: &'a Path,
    ticket_lock: &'a Mutex<()>,
}

fn merge_section(hint: Option<&str>, digest: &[String], shared_files: &[String]) -> String {
    let mut text = format!("{MERGE_HEADING}All items of this stage have run. Reconcile their work and produce the stage outputs.\n\n");
    if let Some(hint) = hint {
        text.push_str(&format!("{hint}\n\n"));
    }
    text.push_str("### Results\n\n");
    text.push_str(&bullet_list(digest));
    if !shared_files.is_empty() {
        text.push_str("\n### Shared files to update now\n\n");
        text.push_str(&bullet_list(shared_files));
    }
    text
}

/// Drives one run from its first stage to a final or paused status.
pub struct RunExec {
    deps: Arc<NativeDeps>,
    store: Arc<RunStore>,
    pipe: Arc<Pipeline>,
    control: Arc<RunControl>,
    dispatcher: Dispatcher,
}

impl RunExec {
    pub fn new(
        deps: Arc<NativeDeps>,
        store: Arc<RunStore>,
        pipe: Arc<Pipeline>,
        control: Arc<RunControl>,
    ) -> Self {
        let dispatcher =
            Dispatcher::new(deps.clone(), store.clone(), pipe.clone(), control.clone());
        RunExec {
            deps,
            store,
            pipe,
            control,
            dispatcher,
        }
    }

    /// Runs until the run is done, paused, waiting for a gate, failed or cancelled.
    pub fn run(&self) -> NativeRunStatus {
        if self.control.is_suspended() {
            return self.store.status();
        }
        if let Err(error) = self.store.set_status(NativeRunStatus::Running, None) {
            warn!(%error, "run status not persisted");
        }
        self.emit(EVENT_RUN_STARTED, None);
        let outcome = self.drive();
        if self.control.is_suspended() {
            return self.store.status();
        }
        let status = match outcome {
            Ok(status) => status,
            Err(error) => {
                let reason = format!("{error:#}");
                if let Err(persist) = self.store.set_status(NativeRunStatus::Failed, Some(reason)) {
                    warn!(%persist, "run failure not persisted");
                }
                NativeRunStatus::Failed
            }
        };
        self.finish(status);
        status
    }

    fn finish(&self, status: NativeRunStatus) {
        if status == NativeRunStatus::Done {
            if let Err(error) = self.store.set_status(NativeRunStatus::Done, None) {
                warn!(%error, "run completion not persisted");
            }
        }
        if status == NativeRunStatus::Cancelled {
            if let Err(error) = self.finalize_cancel() {
                warn!(%error, "cancellation not persisted");
            }
        }
        if let Err(error) = self.store.finish_cost() {
            warn!(%error, "run cost not persisted");
        }
        let event = if status.is_terminal() {
            EVENT_RUN_ENDED
        } else {
            EVENT_RUN_STAGE_CHANGED
        };
        self.emit(event, None);
    }

    /// Cancels a run no thread is executing, and announces it.
    pub fn cancel_idle(&self) -> anyhow::Result<()> {
        self.finalize_cancel()?;
        self.store.finish_cost()?;
        self.emit(EVENT_RUN_ENDED, None);
        Ok(())
    }

    /// Marks every row still running as cancelled and the run as cancelled.
    pub fn finalize_cancel(&self) -> anyhow::Result<()> {
        for row in self.store.view().stages {
            if row.status == NativeStageStatus::Running {
                self.store
                    .cancel_row(&row.name, NativeReason::Cancelled.as_str())?;
            }
        }
        self.store.set_status(
            NativeRunStatus::Cancelled,
            Some(NativeReason::Cancelled.as_str().to_string()),
        )
    }

    /// Compiles the plan pass by pass until it runs out of steps.
    fn drive(&self) -> anyhow::Result<NativeRunStatus> {
        for _ in 0..MAX_COMPILE_PASSES {
            let plan = compile_exec_plan(&self.pipe, &self.store.state(), &DiskPlanSource)?;
            let mut steps = plan.steps.clone();
            if plan.end.is_some() {
                steps.pop();
            }
            let status = self.exec_steps(&steps)?;
            if status != NativeRunStatus::Running {
                return Ok(status);
            }
            match plan.end {
                None => return Ok(NativeRunStatus::Done),
                Some(PassEnd::AwaitingPlan {
                    stage, plan_path, ..
                }) => {
                    if !Path::new(&plan_path).is_file() {
                        return self.halt(
                            NativeRunStatus::Paused,
                            format!(
                                "`{stage}` needs {plan_path}, which its producer did not write"
                            ),
                        );
                    }
                }
                Some(PassEnd::AwaitingComposite { composite, .. }) => {
                    self.store.cross_boundary(&composite)?;
                }
            }
        }
        self.halt(
            NativeRunStatus::Failed,
            "the execution plan did not settle".to_string(),
        )
    }

    fn halt(&self, status: NativeRunStatus, reason: String) -> anyhow::Result<NativeRunStatus> {
        self.store.set_status(status, Some(reason))?;
        Ok(status)
    }

    /// Runs `steps` in order. `Running` means every step went through; any other status stops the walk.
    fn exec_steps(&self, steps: &[Step]) -> anyhow::Result<NativeRunStatus> {
        for step in steps {
            if self.control.is_cancelled() {
                return Ok(NativeRunStatus::Cancelled);
            }
            let status = match step {
                Step::Stage(spec) => self.exec_stage(spec, false)?,
                Step::Parallel { stages } => self.exec_parallel(stages)?,
                Step::Loop {
                    container,
                    condition,
                    source_stage,
                    signal,
                    body_producer,
                    max,
                    continue_when_exhausted,
                    body,
                } => self.exec_loop(
                    container,
                    condition.is_some(),
                    source_stage.as_deref(),
                    signal.as_deref(),
                    body_producer.as_deref(),
                    *max,
                    *continue_when_exhausted,
                    body,
                )?,
                Step::Fanout {
                    stage,
                    items,
                    max,
                    fold,
                    merge_hint,
                } => self.exec_fanout(stage, items, *max, *fold, merge_hint.as_deref())?,
                Step::PlanTasks {
                    stage,
                    plan_path,
                    max,
                    kinds,
                    merge_hint,
                    ..
                } => self.exec_plan_tasks(stage, plan_path, *max, kinds, merge_hint.as_deref())?,
                Step::CompositeBoundary { .. } => NativeRunStatus::Running,
                Step::Skipped { stage, reason } => {
                    self.skip_static(stage, reason)?;
                    NativeRunStatus::Running
                }
            };
            if status != NativeRunStatus::Running {
                return Ok(status);
            }
        }
        Ok(NativeRunStatus::Running)
    }

    fn skip_static(&self, stage: &str, reason: &SkipReason) -> anyhow::Result<()> {
        if self.store.row_status(stage) == Some(NativeStageStatus::Pending) {
            self.store.skip(stage, skip_reason_text(*reason))?;
            self.emit(EVENT_RUN_STAGE_CHANGED, Some(stage));
        }
        Ok(())
    }

    /// `Some(status)` when the row needs no execution: `Running` for done or skipped, else the halt.
    fn already_settled(&self, name: &str) -> anyhow::Result<Option<NativeRunStatus>> {
        let Some(row) = self.store.row(name) else {
            return Ok(None);
        };
        match row.status {
            NativeStageStatus::Done | NativeStageStatus::Skipped => {
                Ok(Some(NativeRunStatus::Running))
            }
            NativeStageStatus::Failed => {
                let reason = row.reason.unwrap_or_default();
                Ok(Some(self.halt(
                    NativeRunStatus::Paused,
                    format!("{name}: {reason}"),
                )?))
            }
            NativeStageStatus::Cancelled => Ok(Some(NativeRunStatus::Cancelled)),
            NativeStageStatus::Pending | NativeStageStatus::Running => Ok(None),
        }
    }

    /// Skips the stage when its runtime `when:` is false. True when it was skipped.
    fn skipped_by_condition(&self, spec: &StageSpec) -> anyhow::Result<bool> {
        let Some(expression) = &spec.when else {
            return Ok(false);
        };
        let Some(stage) = pipeline::find_stage(&self.pipe, &spec.name) else {
            return Ok(false);
        };
        if expr::eval_when(stage, &self.store.state(), &self.pipe) {
            return Ok(false);
        }
        self.store.skip(&spec.name, &when_skip_reason(expression))?;
        self.emit(EVENT_RUN_STAGE_CHANGED, Some(&spec.name));
        Ok(true)
    }

    /// Waits at the stage gate. `Some(status)` stops the walk (cancelled while waiting).
    fn cross_gate(&self, spec: &StageSpec) -> anyhow::Result<Option<NativeRunStatus>> {
        let Some(gate) = &spec.gate else {
            return Ok(None);
        };
        let approved = gate.approved || self.store.is_gate_approved(&spec.name);
        let view = self.store.view();
        let auto = if view.autonomous {
            vec![WILDCARD_GATE.to_string()]
        } else {
            view.request.auto_approve_gates
        };
        if !gate_blocks(true, approved, &spec.name, &auto) {
            if !approved {
                self.store.approve_gate(&spec.name)?;
            }
            return Ok(None);
        }
        self.store.wait_for_gate(&spec.name)?;
        self.emit(EVENT_RUN_WAITING_GATE, Some(&spec.name));
        loop {
            if self.control.is_cancelled() {
                return Ok(Some(NativeRunStatus::Cancelled));
            }
            if self.store.is_gate_approved(&spec.name) {
                self.store.set_status(NativeRunStatus::Running, None)?;
                return Ok(None);
            }
            self.control.wait_for_wake(GATE_POLL);
        }
    }

    /// Runs one plain stage: condition, gate, session, reply, pause rules.
    fn exec_stage(&self, spec: &StageSpec, parallel: bool) -> anyhow::Result<NativeRunStatus> {
        if let Some(status) = self.already_settled(&spec.name)? {
            return Ok(status);
        }
        if self.skipped_by_condition(spec)? {
            return Ok(NativeRunStatus::Running);
        }
        if let Some(status) = self.cross_gate(spec)? {
            return Ok(status);
        }
        self.store.begin(&spec.name)?;
        self.emit(EVENT_RUN_STAGE_CHANGED, Some(&spec.name));
        let mut job = SessionJob {
            stage: spec.clone(),
            label: spec.name.clone(),
            prompt: self.dispatcher.stage_prompt(spec)?,
            reply_dir: self.dispatcher.stage_dir(&spec.name),
            declared_outputs: spec.outputs.clone(),
            guard_scope: is_executor(spec) && !parallel,
            workspace: None,
        };
        let mut run = self.dispatcher.run_job(&job)?;
        if self.store.view().autonomous {
            if let Some(envelope) = blocked_reply(&run) {
                let answers = auto_answers(&envelope.ask_user_questions);
                job.prompt = format!("{}{}", job.prompt, retry_note(&answers));
                let journal = blocked_journal(&spec.name, envelope, answers);
                self.store.add_assumptions(&spec.name, journal)?;
                run = self.dispatcher.run_job(&job)?;
                if blocked_reply(&run).is_some() {
                    return self.fail_stage(&spec.name, NativeReason::BlockedTwice.as_str());
                }
            }
        }
        self.conclude(spec, run)
    }

    /// Records the outcome of a stage session and applies the pause rules.
    fn conclude(&self, spec: &StageSpec, run: SessionRun) -> anyhow::Result<NativeRunStatus> {
        let name = spec.name.as_str();
        let envelope = match run.outcome {
            SessionOutcome::Cancelled => {
                self.store
                    .cancel_row(name, NativeReason::Cancelled.as_str())?;
                return Ok(NativeRunStatus::Cancelled);
            }
            SessionOutcome::Failed(reason) => return self.fail_stage(name, &reason),
            SessionOutcome::Completed(envelope) => envelope,
        };
        self.store
            .settle(name, &envelope, run.usage.as_ref(), run.duration_ms)?;
        let status = EnvelopeStatus::parse(&envelope.status);
        if pauses_run(status, spec.soft_pause) {
            let reason = format!("reply status {}: {}", status.as_str(), envelope.summary);
            return self.fail_stage(name, &reason);
        }
        if let Some(reason) = self.merge_findings(spec, &envelope)? {
            return self.fail_stage(name, &reason);
        }
        if let Some(reason) = self.harness_gap(spec) {
            return self.fail_stage(name, &reason);
        }
        self.emit(EVENT_RUN_STAGE_CHANGED, Some(name));
        Ok(NativeRunStatus::Running)
    }

    fn fail_stage(&self, name: &str, reason: &str) -> anyhow::Result<NativeRunStatus> {
        self.store.fail(name, reason)?;
        self.emit(EVENT_RUN_STAGE_CHANGED, Some(name));
        self.halt(NativeRunStatus::Paused, format!("{name}: {reason}"))
    }

    fn harness_gap(&self, spec: &StageSpec) -> Option<String> {
        if !spec.contracts.contains(&ContractFlag::HarnessGapPause) {
            return None;
        }
        let unevaluated = self
            .store
            .signals(&spec.name)
            .and_then(|signals| signals.get(UNEVALUATED_KEY).and_then(Value::as_u64))
            .unwrap_or(0);
        (unevaluated > 0).then(|| format!("{unevaluated} check(s) could not be evaluated"))
    }

    /// Recomputes the merged findings of a `merge_findings:` stage. `Some(reason)` when it cannot.
    fn merge_findings(
        &self,
        spec: &StageSpec,
        envelope: &NativeReplyEnvelope,
    ) -> anyhow::Result<Option<String>> {
        let Some(merge) = pipeline::find_stage(&self.pipe, &spec.name)
            .and_then(|stage| stage.merge_findings.as_ref())
        else {
            return Ok(None);
        };
        if !envelope.outputs.contains_key(FINDINGS_OUTPUT) {
            return Ok(Some(
                "the merge stage declared no findings output".to_string(),
            ));
        }
        let state = self.store.state();
        let inputs: Vec<MergeInput> = merge
            .from
            .iter()
            .filter_map(|from| {
                let produced = state.stages.iter().find(|stage| &stage.name == from)?;
                let path = produced.outputs.get(FINDINGS_OUTPUT)?;
                Some(MergeInput {
                    label: from.clone(),
                    path: path.clone(),
                })
            })
            .collect();
        if inputs.is_empty() {
            return Ok(None);
        }
        let outcome = match findings::merge(&inputs) {
            Ok(outcome) => outcome,
            Err(error) => return Ok(Some(format!("findings merge: {error}"))),
        };
        let out = self.dispatcher.stage_dir(&spec.name).join(&merge.out);
        std::fs::write(&out, serde_json::to_vec_pretty(&outcome.findings)?)?;
        self.store.set_signal(
            &spec.name,
            HAS_BLOCKING_KEY,
            Value::Bool(outcome.findings.has_blocking_findings),
        )?;
        Ok(None)
    }

    /// Runs adjacent independent stages, at most `max_parallel` at once.
    fn exec_parallel(&self, stages: &[StageSpec]) -> anyhow::Result<NativeRunStatus> {
        let cap = max_parallel(&self.pipe.settings);
        let mut outcome = NativeRunStatus::Running;
        for chunk in stages.chunks(cap) {
            let results: Vec<anyhow::Result<NativeRunStatus>> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|spec| scope.spawn(move || self.exec_stage(spec, true)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("a stage thread panicked")))
                    })
                    .collect()
            });
            for result in results {
                let status = result?;
                if outcome == NativeRunStatus::Running {
                    outcome = status;
                }
            }
            if outcome != NativeRunStatus::Running {
                break;
            }
        }
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_loop(
        &self,
        container: &str,
        has_condition: bool,
        source_stage: Option<&str>,
        signal: Option<&str>,
        body_producer: Option<&str>,
        max: u32,
        continue_when_exhausted: bool,
        body: &[Step],
    ) -> anyhow::Result<NativeRunStatus> {
        if self.store.row_status(container) == Some(NativeStageStatus::Done) {
            return Ok(NativeRunStatus::Running);
        }
        let names = step_names(body);
        let mut iteration = self.store.loop_iteration(container);
        let mut resuming = iteration >= 1 && self.body_unfinished(&names);
        loop {
            if !resuming {
                let holds = self.loop_condition(container, has_condition, source_stage, signal);
                if !loop_continues(holds, iteration, max) {
                    break;
                }
                iteration += 1;
                self.store.set_loop_iteration(container, iteration)?;
                if iteration > 1 {
                    self.store.reset(&self.rows_under(&names))?;
                }
            }
            resuming = false;
            let status = self.exec_steps(body)?;
            if status != NativeRunStatus::Running {
                return Ok(status);
            }
            if let (Some(producer), Some(source)) = (body_producer, source_stage) {
                self.store.copy_signals(producer, source)?;
            }
        }
        let refreshed = body_producer.is_some() && source_stage.is_some();
        if refreshed && self.loop_condition(container, has_condition, source_stage, signal) {
            let reason = format!(
                "`{container}` did not converge after {iteration} iteration(s); blocking findings remain"
            );
            if !continue_when_exhausted {
                return self.halt(NativeRunStatus::Paused, reason);
            }
            warn!("{reason}; `on_exhausted: continue`, the run goes on");
        }
        self.store.complete_container(container)?;
        Ok(NativeRunStatus::Running)
    }

    fn loop_condition(
        &self,
        container: &str,
        has_condition: bool,
        source_stage: Option<&str>,
        signal: Option<&str>,
    ) -> bool {
        if let (Some(source), Some(property)) = (source_stage, signal) {
            let signals = self.store.signals(source);
            return signal_is_set(signals.as_ref().and_then(|found| found.get(property)));
        }
        if !has_condition {
            return true;
        }
        pipeline::find_stage(&self.pipe, container)
            .map(|stage| expr::eval_loop_when(stage, &self.store.state(), &self.pipe))
            .unwrap_or(false)
    }

    /// True when some body row started but not all of them settled: the loop was interrupted mid-iteration.
    fn body_unfinished(&self, names: &[String]) -> bool {
        let rows: Vec<NativeStageStatus> = names
            .iter()
            .filter_map(|name| self.store.row_status(name))
            .collect();
        rows.iter().any(|status| !status.is_settled())
            && rows
                .iter()
                .any(|status| *status != NativeStageStatus::Pending)
    }

    /// The rows named in `names` plus their `<stage>#<id>` sub-rows.
    fn rows_under(&self, names: &[String]) -> Vec<String> {
        self.store
            .view()
            .stages
            .into_iter()
            .map(|row| row.name)
            .filter(|row| {
                names
                    .iter()
                    .any(|name| row == name || row.starts_with(&format!("{name}{ROW_SEPARATOR}")))
            })
            .collect()
    }

    fn fanout_labels(&self, items: &FanoutItems) -> Vec<String> {
        match items {
            FanoutItems::Static(labels) => labels.clone(),
            FanoutItems::FromSignal(reference) => {
                let parsed = reference
                    .strip_prefix(FANOUT_REF_PREFIX)
                    .and_then(|rest| rest.split_once(FANOUT_SIGNALS_MARKER));
                let Some((stage, property)) = parsed else {
                    return Vec::new();
                };
                self.store
                    .signals(stage)
                    .and_then(|signals| signals.get(property).cloned())
                    .and_then(|value| value.as_array().cloned())
                    .map(|list| list.iter().map(value_label).collect())
                    .unwrap_or_default()
            }
        }
    }

    fn exec_fanout(
        &self,
        spec: &StageSpec,
        items: &FanoutItems,
        max: u32,
        fold: bool,
        merge_hint: Option<&str>,
    ) -> anyhow::Result<NativeRunStatus> {
        if let Some(status) = self.already_settled(&spec.name)? {
            return Ok(status);
        }
        if self.skipped_by_condition(spec)? {
            return Ok(NativeRunStatus::Running);
        }
        if let Some(status) = self.cross_gate(spec)? {
            return Ok(status);
        }
        let shards = shard_labels(self.fanout_labels(items), max, fold);
        if shards.is_empty() {
            self.store
                .skip(&spec.name, NativeReason::NoItems.as_str())?;
            return Ok(NativeRunStatus::Running);
        }
        self.store.begin(&spec.name)?;
        self.emit(EVENT_RUN_STAGE_CHANGED, Some(&spec.name));
        let stage_dir = self.dispatcher.stage_dir(&spec.name);
        let base = self.dispatcher.stage_prompt(spec)?;
        let pending: Vec<(usize, Vec<String>)> = shards
            .into_iter()
            .enumerate()
            .filter(|(index, _)| {
                let label = row_name(&spec.name, &format!("{SHARD_PREFIX}{}", index + 1));
                self.store.row_status(&label) != Some(NativeStageStatus::Done)
            })
            .collect();
        let policy = self.pipe.settings.on_item_failure;
        let cap = max_parallel(&self.pipe.settings);
        for chunk in pending.chunks(cap) {
            let results: Vec<anyhow::Result<NativeRunStatus>> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|(index, labels)| {
                        let (stage_dir, base) = (&stage_dir, &base);
                        scope.spawn(move || self.run_shard(spec, *index, labels, stage_dir, base))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("a shard thread panicked")))
                    })
                    .collect()
            });
            for result in results {
                let status = result?;
                if status == NativeRunStatus::Cancelled {
                    return Ok(status);
                }
                if status != NativeRunStatus::Running && policy == ItemFailurePolicy::Pause {
                    return self.halt(
                        NativeRunStatus::Paused,
                        format!("a shard of `{}` failed", spec.name),
                    );
                }
            }
        }
        let digest = self.shard_digest(spec, &stage_dir);
        let prompt = format!("{base}{}", merge_section(merge_hint, &digest, &[]));
        self.run_merge(spec, prompt, &stage_dir)
    }

    /// Runs one shard of a fan-out. `Paused` stands for a failed shard, `Cancelled` for a cancel.
    fn run_shard(
        &self,
        spec: &StageSpec,
        index: usize,
        labels: &[String],
        stage_dir: &Path,
        base: &str,
    ) -> anyhow::Result<NativeRunStatus> {
        let id = format!("{SHARD_PREFIX}{}", index + 1);
        let label = row_name(&spec.name, &id);
        let reply_dir = stage_dir.join(SHARDS_DIR).join(&id);
        let prompt = format!(
            "{base}{SHARD_HEADING}You handle shard {} of the fan-out. Your items: {}.{}",
            index + 1,
            labels.join(LABEL_SEPARATOR),
            reply_override(&reply_dir)
        );
        self.store.begin(&label)?;
        self.emit(EVENT_RUN_STAGE_CHANGED, Some(&label));
        let job = SessionJob {
            stage: spec.clone(),
            label: label.clone(),
            prompt,
            reply_dir,
            declared_outputs: Vec::new(),
            guard_scope: false,
            workspace: None,
        };
        let run = self.dispatcher.run_job(&job)?;
        self.settle_sub_row(&spec.name, &label, run)
    }

    /// Records a plan task or shard outcome on its row. `Running` means it counts as done.
    fn settle_sub_row(
        &self,
        stage: &str,
        label: &str,
        run: SessionRun,
    ) -> anyhow::Result<NativeRunStatus> {
        if matches!(run.outcome, SessionOutcome::Cancelled) {
            self.store
                .cancel_row(label, NativeReason::Cancelled.as_str())?;
            return Ok(NativeRunStatus::Cancelled);
        }
        if let Some(usage) = &run.usage {
            self.store.add_usage(stage, usage)?;
        }
        let envelope = match run.outcome {
            SessionOutcome::Completed(envelope) => envelope,
            SessionOutcome::Failed(reason) => {
                self.store.fail(label, &reason)?;
                self.emit(EVENT_RUN_STAGE_CHANGED, Some(label));
                return Ok(NativeRunStatus::Paused);
            }
            SessionOutcome::Cancelled => return Ok(NativeRunStatus::Cancelled),
        };
        self.store.settle(label, &envelope, None, run.duration_ms)?;
        let status = EnvelopeStatus::parse(&envelope.status);
        let failed = pauses_run(status, true);
        if failed {
            self.store.fail(label, &envelope.summary)?;
        }
        self.emit(EVENT_RUN_STAGE_CHANGED, Some(label));
        Ok(if failed {
            NativeRunStatus::Paused
        } else {
            NativeRunStatus::Running
        })
    }

    fn shard_digest(&self, spec: &StageSpec, stage_dir: &Path) -> Vec<String> {
        self.store
            .view()
            .stages
            .iter()
            .filter(|row| {
                row.name
                    .starts_with(&format!("{}{ROW_SEPARATOR}{SHARD_PREFIX}", spec.name))
            })
            .map(|row| {
                let id = row.name.rsplit(ROW_SEPARATOR).next().unwrap_or_default();
                let dir = stage_dir.join(SHARDS_DIR).join(id);
                self.digest_line(&row.name, row.status, &dir, spec)
            })
            .collect()
    }

    fn digest_line(
        &self,
        label: &str,
        status: NativeStageStatus,
        reply_dir: &Path,
        spec: &StageSpec,
    ) -> String {
        match super::reply::read_reply(reply_dir, spec.agent, &[]) {
            Ok(envelope) => {
                let follow_ups = if envelope.follow_ups.is_empty() {
                    String::new()
                } else {
                    format!(" Follow-ups: {}", envelope.follow_ups.join(" | "))
                };
                format!(
                    "`{label}` {}: {}{follow_ups}",
                    status.as_str(),
                    envelope.summary
                )
            }
            Err(_) => format!("`{label}` {}", status.as_str()),
        }
    }

    /// The merge session of a fan-out or plan stage: it owns the stage's declared outputs.
    fn run_merge(
        &self,
        spec: &StageSpec,
        prompt: String,
        stage_dir: &Path,
    ) -> anyhow::Result<NativeRunStatus> {
        let job = SessionJob {
            stage: spec.clone(),
            label: spec.name.clone(),
            prompt,
            reply_dir: stage_dir.to_path_buf(),
            declared_outputs: spec.outputs.clone(),
            guard_scope: is_executor(spec),
            workspace: None,
        };
        let run = self.dispatcher.run_job(&job)?;
        self.conclude(spec, run)
    }

    fn exec_plan_tasks(
        &self,
        spec: &StageSpec,
        plan_path: &str,
        stage_max: u32,
        kinds: &[String],
        merge_hint: Option<&str>,
    ) -> anyhow::Result<NativeRunStatus> {
        if let Some(status) = self.already_settled(&spec.name)? {
            return Ok(status);
        }
        if self.skipped_by_condition(spec)? {
            return Ok(NativeRunStatus::Running);
        }
        if let Some(status) = self.cross_gate(spec)? {
            return Ok(status);
        }
        let document = plan::load(Path::new(plan_path))?;
        let cap = plan_cap(stage_max, max_parallel(&self.pipe.settings));
        let schedule = plan::schedule(&document, cap as u32, kinds)?;
        self.store
            .record_schedule(schedule.to_state(&spec.name, Path::new(plan_path)))?;
        self.store.begin(&spec.name)?;
        self.emit(EVENT_RUN_STAGE_CHANGED, Some(&spec.name));
        for edge in &schedule.serializations {
            info!(
                before = %edge.before,
                after = %edge.after,
                files = %edge.files.join(LABEL_SEPARATOR),
                "plan tasks serialized on shared files"
            );
        }
        let stage_dir = self.dispatcher.stage_dir(&spec.name);
        let base = self.dispatcher.stage_prompt(spec)?;
        let alone = schedule.tasks.len() == 1;
        let view = self.store.view();
        let workspace = view.workspace;
        let exempt = scope_guard::run_dir_exemption(&workspace, &view.run_dir);
        let stage_baseline =
            scope_guard::capture_task_baseline(&workspace, &PROCESS_RUNNER, exempt.as_deref());
        let settled: Vec<(String, TaskStatus)> = schedule
            .tasks
            .iter()
            .filter(|scheduled| {
                let label = task_label(spec, &scheduled.task.id, alone);
                !alone && self.store.row_status(&label) == Some(NativeStageStatus::Done)
            })
            .map(|scheduled| (scheduled.task.id.clone(), TaskStatus::Success))
            .collect();
        let isolate = task_isolation::should_isolate(alone, schedule.effective_parallel());
        let iso_root = task_isolation::isolation_root(&view.run_dir);
        if isolate {
            task_isolation::sweep(&workspace, &iso_root, &PROCESS_RUNNER);
        }
        let ticket_lock = Mutex::new(());
        let isolation = isolate.then_some(IsolationSetup {
            root: &iso_root,
            ticket_lock: &ticket_lock,
        });
        let slots = PlanSlots::new(schedule.effective_parallel(), settled);
        let halt_on_failure =
            alone || self.pipe.settings.on_item_failure == ItemFailurePolicy::Pause;
        let cancelled = || self.control.is_cancelled();
        let worker = |scheduled: &ScheduledTask| {
            self.run_task(
                spec,
                scheduled,
                &schedule,
                &stage_dir,
                &base,
                alone,
                &slots,
                isolation.as_ref(),
            )
        };
        let (mut outcomes, error) = drive_tasks(
            &slots,
            &schedule.tasks,
            &cancelled,
            halt_on_failure,
            &worker,
        );
        self.record_skipped(spec, &slots, alone)?;
        let overlapped = flag_overlaps(&slots, &schedule.tasks, &mut outcomes);
        self.settle_outcomes(spec, &schedule, &outcomes, alone)?;
        scope_guard::restore_task_violations(
            &workspace,
            &overlapped,
            &stage_baseline,
            &PROCESS_RUNNER,
        )?;
        if let Some(error) = error {
            return Err(error);
        }
        if self.control.is_cancelled()
            || outcomes
                .iter()
                .any(|(_, outcome)| outcome.run_status == NativeRunStatus::Cancelled)
        {
            return Ok(NativeRunStatus::Cancelled);
        }
        let failed = outcomes
            .iter()
            .find(|(_, outcome)| !outcome.is_success())
            .map(|(index, outcome)| (&schedule.tasks[*index], outcome));
        if let Some((scheduled, outcome)) = failed {
            if alone && outcome.run_status != NativeRunStatus::Paused {
                return Ok(outcome.run_status);
            }
            if alone || halt_on_failure {
                let label = task_label(spec, &scheduled.task.id, alone);
                return self.halt(NativeRunStatus::Paused, format!("task `{label}` failed"));
            }
        }
        if alone {
            return Ok(NativeRunStatus::Running);
        }
        let digest = self.task_digest(spec, &schedule.tasks, &stage_dir);
        let prompt = format!(
            "{base}{}",
            merge_section(merge_hint, &digest, &schedule.shared_files)
        );
        self.run_merge(spec, prompt, &stage_dir)
    }

    /// Marks the tasks that never started because a dependency failed.
    fn record_skipped(
        &self,
        spec: &StageSpec,
        slots: &PlanSlots,
        alone: bool,
    ) -> anyhow::Result<()> {
        for id in slots.skipped_ids() {
            let label = task_label(spec, &id, alone);
            self.store
                .skip(&label, NativeReason::DependencyFailed.as_str())?;
            self.store
                .mark_task(&spec.name, &id, TaskStatus::Skipped, None)?;
        }
        Ok(())
    }

    /// Writes the final status of each outcome to its row and to the stage schedule.
    fn settle_outcomes(
        &self,
        spec: &StageSpec,
        schedule: &Schedule,
        outcomes: &[(usize, TaskOutcome)],
        alone: bool,
    ) -> anyhow::Result<()> {
        for (index, outcome) in outcomes {
            let id = schedule.tasks[*index].task.id.as_str();
            let label = task_label(spec, id, alone);
            let failed = outcome.status == TaskStatus::Failed;
            if failed && self.store.row_status(&label) != Some(NativeStageStatus::Failed) {
                let reason = outcome.reason.unwrap_or(NativeReason::TaskFailed);
                self.store
                    .fail(&label, &row_reason(reason, &outcome.violations))?;
                self.emit(EVENT_RUN_STAGE_CHANGED, Some(&label));
            }
            self.store
                .mark_task(&spec.name, id, outcome.status.clone(), outcome.tokens)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn run_task(
        &self,
        spec: &StageSpec,
        scheduled: &ScheduledTask,
        schedule: &Schedule,
        stage_dir: &Path,
        base: &str,
        alone: bool,
        slots: &PlanSlots,
        isolation: Option<&IsolationSetup>,
    ) -> anyhow::Result<TaskOutcome> {
        let id = scheduled.task.id.as_str();
        let label = task_label(spec, id, alone);
        let reply_dir = if alone {
            stage_dir.to_path_buf()
        } else {
            stage_dir.join(TASKS_DIR).join(id)
        };
        let section = task_section(
            scheduled,
            &schedule.shared_files,
            &schedule.serializations,
            alone,
        );
        let mut prompt = format!("{base}{section}");
        if !alone {
            prompt.push_str(&reply_override(&reply_dir));
            self.store.begin(&label)?;
            self.emit(EVENT_RUN_STAGE_CHANGED, Some(&label));
        }
        self.store
            .mark_task(&spec.name, id, TaskStatus::Pending, None)?;
        let view = self.store.view();
        let workspace = view.workspace;
        let exempt = scope_guard::run_dir_exemption(&workspace, &view.run_dir);
        let mut isolated = None;
        if let Some(setup) = isolation {
            let prepared = {
                let _ticket = lock_ticket(setup.ticket_lock);
                task_isolation::prepare(
                    &workspace,
                    setup.root,
                    id,
                    exempt.as_deref(),
                    &PROCESS_RUNNER,
                )
            };
            match prepared {
                Ok(tree) => {
                    isolated = Some(IsolatedTask::new(
                        &workspace,
                        tree,
                        &PROCESS_RUNNER,
                        setup.ticket_lock,
                    ))
                }
                Err(error) => {
                    warn!(task = id, %error, "could not prepare an isolated worktree");
                    let reason = NativeReason::IsolationFailed;
                    self.store.fail(&label, &row_reason(reason, &[]))?;
                    self.emit(EVENT_RUN_STAGE_CHANGED, Some(&label));
                    return Ok(TaskOutcome::failed(reason));
                }
            }
        }
        let task_workspace = isolated
            .as_ref()
            .map_or(workspace.as_path(), |task| task.tree.path.as_path());
        let baseline = (!alone).then(|| {
            let exempt = exempt.as_deref().filter(|_| isolated.is_none());
            scope_guard::capture_task_baseline(task_workspace, &PROCESS_RUNNER, exempt)
        });
        let job = SessionJob {
            stage: spec.clone(),
            label: label.clone(),
            prompt,
            reply_dir,
            declared_outputs: if alone {
                spec.outputs.clone()
            } else {
                Vec::new()
            },
            guard_scope: alone && is_executor(spec),
            workspace: isolated.as_ref().map(|task| task.tree.path.clone()),
        };
        let mut run = self.dispatcher.run_job(&job)?;
        let tokens = run.usage.as_ref().map(|usage| usage.output);
        // An isolated task is checked and applied back before its row is marked done, so a crash
        // in between never leaves a done row whose changes were lost.
        let mut decided = None;
        if let (Some(task), Some(baseline), Some(setup), true) = (
            isolated.as_ref(),
            baseline.as_ref(),
            isolation,
            session_succeeded(&run),
        ) {
            let mut outcome = outcome_of(NativeRunStatus::Running, tokens);
            verify_task_scope(
                task_workspace,
                scheduled,
                schedule,
                slots,
                baseline,
                true,
                &mut outcome,
            )?;
            if outcome.is_success() {
                let _ticket = lock_ticket(setup.ticket_lock);
                apply_task(&workspace, &task.tree, &mut outcome, &PROCESS_RUNNER);
            }
            if !outcome.is_success() {
                let reason = outcome.reason.unwrap_or(NativeReason::TaskFailed);
                run.outcome = SessionOutcome::Failed(row_reason(reason, &outcome.violations));
                outcome.tokens = tokens;
            }
            decided = Some(outcome);
        }
        let status = if alone {
            self.conclude(spec, run)?
        } else {
            self.settle_sub_row(&spec.name, &label, run)?
        };
        let settled_failed = decided
            .as_ref()
            .is_some_and(|outcome| !outcome.is_success());
        let mut outcome = decided.unwrap_or_else(|| outcome_of(status, tokens));
        if let (true, Some(baseline), None) =
            (outcome.is_success(), baseline.as_ref(), isolated.as_ref())
        {
            verify_task_scope(
                task_workspace,
                scheduled,
                schedule,
                slots,
                baseline,
                false,
                &mut outcome,
            )?;
        }
        if outcome.status == TaskStatus::Failed && !alone && !settled_failed {
            if let Some(reason) = outcome.reason {
                self.store
                    .fail(&label, &row_reason(reason, &outcome.violations))?;
                self.emit(EVENT_RUN_STAGE_CHANGED, Some(&label));
            }
        }
        Ok(outcome)
    }

    fn task_digest(
        &self,
        spec: &StageSpec,
        tasks: &[ScheduledTask],
        stage_dir: &Path,
    ) -> Vec<String> {
        tasks
            .iter()
            .map(|scheduled| {
                let label = row_name(&spec.name, &scheduled.task.id);
                let status = self
                    .store
                    .row_status(&label)
                    .unwrap_or(NativeStageStatus::Pending);
                let dir = stage_dir.join(TASKS_DIR).join(&scheduled.task.id);
                self.digest_line(&label, status, &dir, spec)
            })
            .collect()
    }

    /// Publishes a run event; `stage` adds the row's status and reason.
    fn emit(&self, event: &str, stage: Option<&str>) {
        let view = self.store.view();
        let row = stage.and_then(|name| view.stage(name));
        let payload = NativeRunEvent {
            run_id: view.run_id.clone(),
            template_id: view.template_id.clone(),
            status: view.status,
            stage: stage.map(str::to_string),
            stage_status: row.map(|found| found.status),
            reason: row
                .and_then(|found| found.reason.clone())
                .or_else(|| view.reason.clone()),
        };
        self.deps
            .bus
            .emit(event, serde_json::to_value(&payload).unwrap_or(Value::Null));
    }
}

/// Lock key that keeps a run on a single executor thread.
pub fn run_lock_key(run_id: &str) -> String {
    format!("native-run:{run_id}")
}

#[cfg(test)]
#[path = "../../tests/unit/exec_loop_tests.rs"]
mod tests;
