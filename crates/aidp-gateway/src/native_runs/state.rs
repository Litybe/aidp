//! Native run state: `run-state.json` written through aidp-core, `native-run.json` beside it.
//! Every transition goes through [`RunStore`], so both files and the vault notes stay in step.

use super::agent_board::AgentBoard;
use super::autonomy::{reply_assumptions, reply_escalations};
use crate::models::constants::native_shutdown::SUSPENDED_STORE;
use crate::models::enums::native_reason::NativeReason;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::native_stage_status::NativeStageStatus;
use crate::models::enums::PermissionMode;
use crate::models::native_reply_envelope::NativeReplyEnvelope;
use crate::models::native_run_start::StartNativeRun;
use crate::models::native_run_view::NativeRunView;
use crate::models::native_stage_view::NativeStageView;
use crate::store::write_json;
use crate::sync::lock;
use aidp_core::clock::{Clock, SystemClock};
use aidp_core::cost::PriceTable;
use aidp_core::models::assumption::Assumption;
use aidp_core::plan::{ScheduleState, TaskStatus};
use aidp_core::run::{self, RunCost, RunState, StageStatus, StageUsage};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// File holding the [`NativeRunView`] of a run, next to `run-state.json`.
pub const NATIVE_RUN_FILE: &str = "native-run.json";

const UNKNOWN_MODEL: &str = "unknown";

/// What the executor thread of a run shares with the API: cancellation, live sessions, gate wake-ups.
#[derive(Default)]
pub struct RunControl {
    cancelled: AtomicBool,
    suspended: AtomicBool,
    permission_fallback: AtomicBool,
    live_sessions: Mutex<HashSet<String>>,
    gate_wait: Mutex<()>,
    gate_changed: Condvar,
    /// Agent jobs a host runs for this run; empty unless the run uses external agents.
    pub board: AgentBoard,
}

impl RunControl {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || self.is_suspended()
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended.load(Ordering::SeqCst)
    }

    /// Stops executor loops without turning a restart into a user cancellation.
    pub fn suspend(&self) {
        self.suspended.store(true, Ordering::SeqCst);
        self.wake();
    }

    /// Flags the run cancelled and wakes a thread parked on a gate.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.wake();
    }

    /// Explicit policy always wins; an unspecified policy keeps the historical fallback.
    pub fn permission_mode(&self, requested: Option<PermissionMode>) -> PermissionMode {
        if let Some(requested) = requested {
            requested
        } else if self.permission_fallback.load(Ordering::SeqCst) {
            PermissionMode::STAGE_FALLBACK
        } else {
            PermissionMode::STAGE_PREFERRED
        }
    }

    /// Makes every later stage session of the run ask for the fallback permission mode.
    pub fn fall_back_permission(&self) {
        self.permission_fallback.store(true, Ordering::SeqCst);
    }

    pub fn track_session(&self, session_id: &str) {
        lock(&self.live_sessions).insert(session_id.to_string());
    }

    pub fn untrack_session(&self, session_id: &str) {
        lock(&self.live_sessions).remove(session_id);
    }

    pub fn live_sessions(&self) -> Vec<String> {
        lock(&self.live_sessions).iter().cloned().collect()
    }

    /// Wakes the thread waiting on a gate, after an approval or a cancel.
    pub fn wake(&self) {
        let _guard = lock(&self.gate_wait);
        self.gate_changed.notify_all();
    }

    /// Parks the caller until [`RunControl::wake`] or `timeout`.
    pub fn wait_for_wake(&self, timeout: Duration) {
        let guard = lock(&self.gate_wait);
        let _ = self.gate_changed.wait_timeout(guard, timeout);
    }
}

struct Inner {
    frozen: bool,
    state: RunState,
    view: NativeRunView,
}

/// The single writer of a run's two state files.
pub struct RunStore {
    inner: Mutex<Inner>,
}

fn now() -> String {
    SystemClock.now_rfc3339()
}

fn persist(inner: &Inner) -> anyhow::Result<()> {
    run::save(&inner.state)?;
    write_json(&inner.view.run_dir.join(NATIVE_RUN_FILE), &inner.view)
}

fn state_stage<'a>(state: &'a mut RunState, name: &str) -> Option<&'a mut run::StageState> {
    state.stages.iter_mut().find(|stage| stage.name == name)
}

impl RunStore {
    /// Wraps a freshly initialised run, with one pending row per stage.
    pub fn create(
        state: RunState,
        request: &StartNativeRun,
        workspace: PathBuf,
    ) -> anyhow::Result<RunStore> {
        let mut state = state;
        state.autonomous = request.autonomous;
        let view = NativeRunView {
            run_id: state.run_id.clone(),
            template_id: state.pipeline_id.clone(),
            status: NativeRunStatus::Pending,
            reason: None,
            workspace,
            run_dir: state.run_dir.clone(),
            request: request.clone(),
            pending_gate: None,
            started_at: state.started_at.clone(),
            ended_at: None,
            plan: state.plan.clone(),
            stages: state
                .stages
                .iter()
                .map(|stage| NativeStageView::pending(&stage.name))
                .collect(),
            autonomous: request.autonomous,
            assumptions: Vec::new(),
            escalations: Vec::new(),
        };
        let inner = Inner {
            frozen: false,
            state,
            view,
        };
        persist(&inner)?;
        Ok(RunStore {
            inner: Mutex::new(inner),
        })
    }

    /// Loads a run from its directory.
    pub fn open(run_dir: &Path) -> anyhow::Result<RunStore> {
        let state_text = std::fs::read_to_string(run_dir.join(run::RUN_STATE_FILE))?;
        let view_text = std::fs::read_to_string(run_dir.join(NATIVE_RUN_FILE))?;
        let state: RunState = serde_json::from_str(&state_text)?;
        let mut view: NativeRunView = serde_json::from_str(&view_text)?;
        view.autonomous = state.autonomous || view.request.autonomous;
        view.assumptions = state.assumptions.clone();
        view.escalations = state.escalations.clone();
        Ok(RunStore {
            inner: Mutex::new(Inner {
                frozen: false,
                state,
                view,
            }),
        })
    }

    pub fn run_id(&self) -> String {
        lock(&self.inner).view.run_id.clone()
    }

    pub fn view(&self) -> NativeRunView {
        lock(&self.inner).view.clone()
    }

    pub fn state(&self) -> RunState {
        lock(&self.inner).state.clone()
    }

    /// Keeps the last durable state intact while stopped executor threads unwind.
    pub fn freeze(&self) {
        lock(&self.inner).frozen = true;
    }

    fn update<R>(
        &self,
        change: impl FnOnce(&mut RunState, &mut NativeRunView) -> R,
    ) -> anyhow::Result<R> {
        let mut inner = lock(&self.inner);
        anyhow::ensure!(!inner.frozen, SUSPENDED_STORE);
        let Inner { state, view, .. } = &mut *inner;
        let result = change(state, view);
        persist(&inner)?;
        Ok(result)
    }

    pub fn status(&self) -> NativeRunStatus {
        lock(&self.inner).view.status
    }

    /// Sets the run status and its reason; a terminal status also stamps `ended_at`.
    pub fn set_status(
        &self,
        status: NativeRunStatus,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        self.update(|_, view| {
            view.status = status;
            view.reason = reason;
            if status.is_terminal() {
                view.ended_at = Some(now());
            } else {
                view.ended_at = None;
            }
        })
    }

    /// Sets the model the remaining stages of the run use.
    pub fn set_model(&self, model: &str) -> anyhow::Result<()> {
        self.update(|_, view| view.request.model = Some(model.to_string()))
    }

    pub fn row_status(&self, name: &str) -> Option<NativeStageStatus> {
        lock(&self.inner).view.stage(name).map(|row| row.status)
    }

    pub fn row(&self, name: &str) -> Option<NativeStageView> {
        lock(&self.inner).view.stage(name).cloned()
    }

    /// Marks a row running; the state file shows plain stages as `in_progress`.
    pub fn begin(&self, name: &str) -> anyhow::Result<()> {
        self.update(|state, view| {
            let row = view.stage_or_insert(name);
            row.status = NativeStageStatus::Running;
            row.reason = None;
            row.finished_at = None;
            row.started_at.get_or_insert_with(now);
            if let Some(stage) = state_stage(state, name) {
                stage.status = StageStatus::InProgress;
                stage.started_at.get_or_insert_with(now);
            }
        })
    }

    /// Records a launched session on its row.
    pub fn attach_session(&self, name: &str, session_id: &str) -> anyhow::Result<()> {
        self.update(|_, view| {
            let row = view.stage_or_insert(name);
            row.attempts += 1;
            row.session_ids.push(session_id.to_string());
        })
    }

    /// Accepts a reply: the row is done and the stage state takes the envelope.
    pub fn settle(
        &self,
        name: &str,
        envelope: &NativeReplyEnvelope,
        usage: Option<&StageUsage>,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        self.update(|state, view| {
            let row = view.stage_or_insert(name);
            row.status = NativeStageStatus::Done;
            row.reason = None;
            row.finished_at = Some(now());
            row.signals = envelope.signals.clone();
            if let Some(used) = usage {
                row.cost_usd = Some(row.cost_usd.unwrap_or(0.0) + used.cost_usd.unwrap_or(0.0));
            }
            if let Some(stage) = state_stage(state, name) {
                apply_envelope(stage, envelope, usage, duration_ms);
            }
            let journaled = reply_assumptions(name, envelope);
            state.assumptions.extend(journaled.iter().cloned());
            view.assumptions.extend(journaled);
            let escalated = reply_escalations(name, envelope);
            state.escalations.extend(escalated.iter().cloned());
            view.escalations.extend(escalated);
        })
    }

    /// Journals the auto-answered questions of `stage` in the run state and the view.
    pub fn add_assumptions(&self, stage: &str, answers: Vec<Assumption>) -> anyhow::Result<()> {
        self.update(|state, view| {
            for mut answer in answers {
                answer.stage = stage.to_string();
                state.assumptions.push(answer.clone());
                view.assumptions.push(answer);
            }
        })
    }

    /// Fails a row with the reason a human will read.
    pub fn fail(&self, name: &str, reason: &str) -> anyhow::Result<()> {
        self.settle_without_reply(name, NativeStageStatus::Failed, reason, StageStatus::Cancel)
    }

    /// Marks a row cancelled, keeping the stage state `cancel` like a failed one.
    pub fn cancel_row(&self, name: &str, reason: &str) -> anyhow::Result<()> {
        self.settle_without_reply(
            name,
            NativeStageStatus::Cancelled,
            reason,
            StageStatus::Cancel,
        )
    }

    fn settle_without_reply(
        &self,
        name: &str,
        status: NativeStageStatus,
        reason: &str,
        stage_status: StageStatus,
    ) -> anyhow::Result<()> {
        self.update(|state, view| {
            let row = view.stage_or_insert(name);
            row.status = status;
            row.reason = Some(reason.to_string());
            row.finished_at = Some(now());
            if let Some(stage) = state_stage(state, name) {
                stage.status = stage_status;
                stage.finished_at = Some(now());
            }
        })
    }

    /// Skips a stage whose condition is false or whose dependency failed.
    pub fn skip(&self, name: &str, reason: &str) -> anyhow::Result<()> {
        let mut inner = lock(&self.inner);
        anyhow::ensure!(!inner.frozen, SUSPENDED_STORE);
        let row = inner.view.stage_or_insert(name);
        row.status = NativeStageStatus::Skipped;
        row.reason = Some(reason.to_string());
        row.finished_at = Some(now());
        if inner.state.stages.iter().any(|stage| stage.name == name) {
            run::stage_skip(&mut inner.state, name)?;
        }
        persist(&inner)
    }

    /// Marks a loop container done without a session: its body already ran.
    pub fn complete_container(&self, name: &str) -> anyhow::Result<()> {
        let mut inner = lock(&self.inner);
        anyhow::ensure!(!inner.frozen, SUSPENDED_STORE);
        let row = inner.view.stage_or_insert(name);
        row.status = NativeStageStatus::Done;
        row.finished_at = Some(now());
        if inner.state.stages.iter().any(|stage| stage.name == name) {
            run::stage_complete_noop(&mut inner.state, name)?;
        }
        persist(&inner)
    }

    /// Puts rows back to pending, for the next iteration of a loop or a retry.
    pub fn reset(&self, names: &[String]) -> anyhow::Result<()> {
        self.update(|state, view| {
            for name in names {
                if let Some(row) = view.stages.iter_mut().find(|row| &row.name == name) {
                    row.status = NativeStageStatus::Pending;
                    row.reason = None;
                    row.finished_at = None;
                    row.signals = None;
                }
                if let Some(stage) = state_stage(state, name) {
                    stage.status = StageStatus::Pending;
                    stage.finished_at = None;
                }
            }
        })
    }

    pub fn signals(&self, name: &str) -> Option<Value> {
        lock(&self.inner)
            .view
            .stage(name)
            .and_then(|row| row.signals.clone())
    }

    /// Replaces the signals of `target` with those of `source`, as a correction loop refreshes its source.
    pub fn copy_signals(&self, source: &str, target: &str) -> anyhow::Result<()> {
        self.update(|_, view| {
            let signals = view.stage(source).and_then(|row| row.signals.clone());
            view.stage_or_insert(target).signals = signals;
        })
    }

    /// Overwrites one signal, used when findings are merged again after the stage.
    pub fn set_signal(&self, name: &str, key: &str, value: Value) -> anyhow::Result<()> {
        self.update(|_, view| {
            let row = view.stage_or_insert(name);
            let mut signals = match row.signals.take() {
                Some(Value::Object(map)) => map,
                _ => serde_json::Map::new(),
            };
            signals.insert(key.to_string(), value);
            row.signals = Some(Value::Object(signals));
        })
    }

    pub fn loop_iteration(&self, container: &str) -> u32 {
        lock(&self.inner)
            .state
            .loops
            .get(container)
            .map(|state| state.iter)
            .unwrap_or(0)
    }

    pub fn set_loop_iteration(&self, container: &str, iteration: u32) -> anyhow::Result<()> {
        self.update(|state, _| {
            state.loops.entry(container.to_string()).or_default().iter = iteration;
        })
    }

    /// Records a gate as waiting for a human.
    pub fn wait_for_gate(&self, stage: &str) -> anyhow::Result<()> {
        let mut inner = lock(&self.inner);
        anyhow::ensure!(!inner.frozen, SUSPENDED_STORE);
        run::record_resume(&mut inner.state, None, &[], Some(stage.to_string()), None)?;
        inner.view.pending_gate = Some(stage.to_string());
        inner.view.status = NativeRunStatus::WaitingGate;
        inner.view.reason = Some(stage.to_string());
        persist(&inner)
    }

    /// Records a gate as approved and clears the pending one.
    pub fn approve_gate(&self, stage: &str) -> anyhow::Result<()> {
        let mut inner = lock(&self.inner);
        anyhow::ensure!(!inner.frozen, SUSPENDED_STORE);
        run::record_resume(&mut inner.state, None, &[stage.to_string()], None, None)?;
        if inner.view.pending_gate.as_deref() == Some(stage) {
            inner.view.pending_gate = None;
        }
        persist(&inner)
    }

    pub fn is_gate_approved(&self, stage: &str) -> bool {
        run::approved_gates(&lock(&self.inner).state)
            .iter()
            .any(|approved| approved == stage)
    }

    /// Records a composite as crossed, so the next compilation goes through it.
    pub fn cross_boundary(&self, composite: &str) -> anyhow::Result<()> {
        let mut inner = lock(&self.inner);
        anyhow::ensure!(!inner.frozen, SUSPENDED_STORE);
        run::record_resume(
            &mut inner.state,
            None,
            &[],
            None,
            Some(composite.to_string()),
        )?;
        persist(&inner)
    }

    /// Stores the schedule of a plan-driven stage, keeping outcomes already recorded.
    pub fn record_schedule(&self, schedule: ScheduleState) -> anyhow::Result<()> {
        let mut inner = lock(&self.inner);
        anyhow::ensure!(!inner.frozen, SUSPENDED_STORE);
        run::record_plan_schedules(&mut inner.state, vec![schedule])?;
        persist(&inner)
    }

    /// Sets the outcome of one plan task in the stage schedule.
    pub fn mark_task(
        &self,
        stage: &str,
        task_id: &str,
        status: TaskStatus,
        tokens: Option<u64>,
    ) -> anyhow::Result<()> {
        self.update(|state, _| {
            let task = state
                .plan_schedules
                .get_mut(stage)
                .and_then(|schedule| schedule.tasks.iter_mut().find(|task| task.id == task_id));
            if let Some(task) = task {
                task.status = status;
                task.tokens_used = tokens;
            }
        })
    }

    /// Adds the usage of an extra session to a stage, e.g. a plan task or a repair.
    pub fn add_usage(&self, stage: &str, usage: &StageUsage) -> anyhow::Result<()> {
        self.update(|state, view| {
            if let Some(row) = view.stages.iter_mut().find(|row| row.name == stage) {
                row.cost_usd = Some(row.cost_usd.unwrap_or(0.0) + usage.cost_usd.unwrap_or(0.0));
            }
            if let Some(found) = state_stage(state, stage) {
                found.usage = Some(merge_usage(found.usage.take(), usage));
            }
        })
    }

    /// Rolls the per-stage usage up into the run cost, as `run-state.json` carries it.
    pub fn finish_cost(&self) -> anyhow::Result<()> {
        self.update(|state, _| {
            state.cost = Some(rollup_cost(state));
        })
    }

    /// Puts running rows back to pending for a restart, or fails a row interrupted a second time.
    /// Returns the rows that will run again.
    pub fn prepare_resume(&self) -> anyhow::Result<Vec<String>> {
        self.update(|state, view| {
            let mut again = Vec::new();
            for row in view.stages.iter_mut() {
                if row.status != NativeStageStatus::Running {
                    continue;
                }
                if row.resumed {
                    row.status = NativeStageStatus::Failed;
                    row.reason = Some(NativeReason::InterruptedTwice.to_string());
                    row.finished_at = Some(now());
                    if let Some(stage) = state_stage(state, &row.name) {
                        stage.status = StageStatus::Cancel;
                    }
                    continue;
                }
                row.resumed = true;
                row.status = NativeStageStatus::Pending;
                if let Some(stage) = state_stage(state, &row.name) {
                    stage.status = StageStatus::Pending;
                }
                again.push(row.name.clone());
            }
            again
        })
    }
}

fn apply_envelope(
    stage: &mut run::StageState,
    envelope: &NativeReplyEnvelope,
    usage: Option<&StageUsage>,
    duration_ms: u64,
) {
    let succeeded = aidp_core::run::EnvelopeStatus::parse(&envelope.status).is_success();
    stage.status = if succeeded {
        StageStatus::Success
    } else {
        StageStatus::Cancel
    };
    stage.finished_at = Some(now());
    stage.duration_ms = Some(stage.duration_ms.unwrap_or(0) + duration_ms);
    for (name, path) in &envelope.outputs {
        stage.outputs.insert(name.clone(), PathBuf::from(path));
    }
    let summary = envelope.summary.trim();
    if !summary.is_empty() {
        stage.summary = Some(summary.to_string());
    }
    if !envelope.follow_ups.is_empty() {
        stage.follow_ups = envelope.follow_ups.clone();
    }
    if let Some(used) = usage {
        stage.tokens_used = Some(stage.tokens_used.unwrap_or(0) + used.output);
        stage.usage = Some(merge_usage(stage.usage.take(), used));
    }
}

fn merge_usage(previous: Option<StageUsage>, added: &StageUsage) -> StageUsage {
    let Some(mut total) = previous else {
        return added.clone();
    };
    total.input += added.input;
    total.output += added.output;
    total.cache_creation += added.cache_creation;
    total.cache_read += added.cache_read;
    total.cost_usd = match (total.cost_usd, added.cost_usd) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0.0) + right.unwrap_or(0.0)),
    };
    total
}

fn rollup_cost(state: &RunState) -> RunCost {
    let prices = PriceTable::from_env_or_embedded();
    let mut by_model: BTreeMap<String, StageUsage> = BTreeMap::new();
    let mut total_usd: Option<f64> = None;
    let mut tokens_billable = 0;
    for usage in state.stages.iter().filter_map(|stage| stage.usage.as_ref()) {
        tokens_billable += usage.input + usage.output + usage.cache_creation + usage.cache_read;
        if let Some(cost) = usage.cost_usd {
            total_usd = Some(total_usd.unwrap_or(0.0) + cost);
        }
        let model = usage
            .model
            .clone()
            .unwrap_or_else(|| UNKNOWN_MODEL.to_string());
        let merged = merge_usage(by_model.remove(&model), usage);
        by_model.insert(model, merged);
    }
    RunCost {
        total_usd,
        tokens_billable,
        pricing_version: Some(prices.pricing_version().to_string()),
        captured_at: now(),
        by_model,
    }
}
