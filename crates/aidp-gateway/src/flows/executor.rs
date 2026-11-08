//! Effect loop: run what `advance` says, feed the result back.
//! State is written before the effect; an applied id is never replayed.

use super::action_processes::FlowActionProcesses;
use super::events;
use super::facts;
use super::lock::{workspace_key, WorkspaceGuard};
use super::map_flow_error;
use super::pipeline_inputs;
use super::recovery::FlowWorker;
use super::registry::{
    ActionContext, PipelineLaunch, ProbeContext, Registry, SessionStart, SessionState,
    DEFAULT_AUTONOMOUS, WITH_AUTONOMOUS, WITH_AUTO_APPROVE_GATES, WITH_ENGINE,
};
use super::store::{is_active, FlowStore, InboxItem, JournalLine, Runtime, WaitRecord};
use super::FlowEngineError;
use crate::models::action_job::ID_SEPARATOR;
use crate::models::constants::flow_facts::LIVE_STATUS_TIMEOUT;
use crate::models::constants::flow_manual::WORKSPACE_BUSY;
use crate::models::constants::ticket_flow::{STEP_MERGE, STEP_READY, TICKETS_DIR};
use crate::models::enums::agent_executor::AgentExecutor;
use crate::models::enums::engine::Engine;
use crate::models::enums::FlowTicketReason;
use crate::models::pr_workspace_holder::PrWorkspaceHolder;
pub use crate::models::Ports;
use crate::models::{ActionJob, ExternalManualEntry};
use crate::pr_workspace_holder;
use crate::sync::lock;
use crate::threads;
use crate::time::now_iso_with;
use crate::unity_lock::{self, Acquire, Change, ResourceHolder, ResourceManager};
use aidp_flows::models::constants::resource;
use aidp_flows::{
    advance, effect_id, ActionOutcome, Effect, EffectKind, EffectName, Event, FlowState,
    FlowStatus, HumanDecision, PipelineRunStatus, Step, StepKind, StepReason, StepStatus,
    MANUAL_PREFIX,
};
use aidp_tickets::ledger;
use aidp_tickets::model::{guard, Actor, Status};
use aidp_tickets::store::TicketStore;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[path = "project_checkout.rs"]
mod project_checkout;

const LOCK_POLL: Duration = Duration::from_millis(25);
const FLOWS_ACTION_LOOP: &str = "flows-action";
const PANIC_MESSAGE: &str = "action panicked";
const UNKNOWN_ACTION_PREFIX: &str = "unknown action";
/// Bus event published when a ticket reaches its token or Pixellab ceiling.
pub const TICKET_BUDGET_EVENT: &str = "ticket.budget_exceeded";
const NO_CHECKOUT_MESSAGE: &str = "the branch of the ticket could not be checked out again";
const NO_PROJECT_MESSAGE: &str = "no project matches the ticket";

pub struct Executor {
    pub(super) store: FlowStore,
    registry: Registry,
    pub(super) ports: Ports,
    key_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    inflight: Mutex<HashSet<String>>,
    stopping: AtomicBool,
    transitions: RwLock<()>,
    action_admission: Mutex<()>,
    action_workers: Mutex<Vec<JoinHandle<()>>>,
    action_processes: FlowActionProcesses,
    resources: ResourceManager,
}

/// Escapes every non-ASCII character as `\uXXXX`, so the file survives any shell or code page.
pub fn ascii_json(value: &Value) -> String {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii() {
            escaped.push(character);
            continue;
        }
        let mut units = [0u16; 2];
        for unit in character.encode_utf16(&mut units) {
            let _ = write!(escaped, "\\u{unit:04x}");
        }
    }
    escaped
}

fn file_safe(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The ticket status a delivery run implies, or `None` once the run is aborted.
fn ticket_status_of(state: &FlowState) -> Option<Status> {
    let step_done = |id: &str| {
        state
            .steps
            .get(id)
            .is_some_and(|step| step.status == StepStatus::Done)
    };
    match state.status {
        FlowStatus::Aborted => None,
        _ if step_done(STEP_MERGE) => Some(Status::Done),
        FlowStatus::Escalated => Some(Status::Blocked),
        _ if step_done(STEP_READY) => Some(Status::InReview),
        _ => Some(Status::InProgress),
    }
}

/// The step the machine is positioned on: the top frame's step at its index.
pub(super) fn step_at_cursor(state: &FlowState) -> Option<Step> {
    let frame = state.stack.last()?;
    frame.steps.get(frame.index).cloned()
}

fn gates_of(with: &BTreeMap<String, Value>) -> Option<Vec<String>> {
    let list = with.get(WITH_AUTO_APPROVE_GATES)?.as_array()?;
    Some(
        list.iter()
            .filter_map(|gate| gate.as_str().map(str::to_string))
            .collect(),
    )
}

impl Executor {
    pub fn new(store: FlowStore, registry: Registry, ports: Ports) -> Arc<Self> {
        Arc::new(Executor {
            action_processes: FlowActionProcesses::new(ports.runner.clone()),
            store,
            registry,
            ports,
            key_locks: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashSet::new()),
            stopping: AtomicBool::new(false),
            transitions: RwLock::new(()),
            action_admission: Mutex::new(()),
            action_workers: Mutex::new(Vec::new()),
            resources: ResourceManager::default(),
        })
    }

    pub(super) fn require_running(&self) -> anyhow::Result<()> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err(crate::session_error::SessionError::ShuttingDown.into());
        }
        Ok(())
    }

    pub(super) fn transition(&self) -> anyhow::Result<RwLockReadGuard<'_, ()>> {
        self.require_running()?;
        let guard = self
            .transitions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.require_running()?;
        Ok(guard)
    }

    pub(super) fn shutdown(&self) -> anyhow::Result<()> {
        self.stopping.store(true, Ordering::SeqCst);
        {
            let _transitions = self
                .transitions
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _admission = lock(&self.action_admission);
            self.store.freeze();
        }
        self.action_processes.shutdown()?;
        let workers = std::mem::take(&mut *lock(&self.action_workers));
        let mut join_failed = false;
        for worker in workers {
            join_failed |= worker.join().is_err();
        }
        anyhow::ensure!(
            !join_failed,
            crate::models::constants::flow_shutdown::JOIN_FAILED
        );
        self.action_processes.shutdown()
    }

    /// True when no action is in flight and no run is being transitioned.
    pub fn is_idle(&self) -> bool {
        if !lock(&self.inflight).is_empty() {
            return false;
        }
        lock(&self.key_locks)
            .values()
            .all(|guard| !matches!(guard.try_lock(), Err(std::sync::TryLockError::WouldBlock)))
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn now(&self) -> String {
        self.ports.clock.now_rfc3339()
    }

    pub(super) fn key_lock(&self, key: &str) -> Arc<Mutex<()>> {
        lock(&self.key_locks)
            .entry(key.to_string())
            .or_default()
            .clone()
    }

    /// The repo segment and state of run `key`.
    pub fn load(&self, key: &str) -> anyhow::Result<(String, FlowState)> {
        let not_found = || FlowEngineError::NotFound(format!("no flow run for `{key}`"));
        let repo = self.store.index_get(key).ok_or_else(not_found)?.repo;
        let state = self.store.load_state(&repo, key).ok_or_else(not_found)?;
        Ok((repo, state))
    }

    /// Saves a run that has not started yet and starts it, under the run's lock.
    pub fn start_run(
        self: &Arc<Self>,
        repo: &str,
        state: FlowState,
        ticket: Value,
    ) -> anyhow::Result<FlowState> {
        let _transition = self.transition()?;
        let key = state.key.clone();
        let guard = self.key_lock(&key);
        let _held = lock(&guard);
        self.store.save_state(repo, &state, &self.now())?;
        self.deliver_locked(
            &key,
            Event::Start {
                ticket,
                at: self.now(),
            },
        )
    }

    /// Applies `event` to run `key` and performs the resulting effects.
    pub fn deliver(self: &Arc<Self>, key: &str, event: Event) -> anyhow::Result<FlowState> {
        let _transition = self.transition()?;
        let guard = self.key_lock(key);
        let _held = lock(&guard);
        self.deliver_locked(key, event)
    }

    /// Like [`Self::deliver`] for results coming back from workers: a stale
    /// or refused result is dropped, not reported.
    fn deliver_quiet(self: &Arc<Self>, key: &str, event: Event) {
        if let Err(error) = self.deliver(key, event) {
            tracing::debug!(key, %error, "flow event dropped");
        }
    }

    pub(super) fn deliver_locked(
        self: &Arc<Self>,
        key: &str,
        event: Event,
    ) -> anyhow::Result<FlowState> {
        let (repo, state) = self.load(key)?;
        self.raise_ceiling_on_approval(&state, &event);
        let definition = state.def.clone();
        let (next, effects) = advance(&definition, state, event).map_err(map_flow_error)?;
        self.store.save_state(&repo, &next, &self.now())?;
        self.sync_ticket_status(&next);
        self.sync_resources(&next);
        self.end_project_checkout(&repo, &next);
        self.apply_effects(&repo, key, effects)?;
        self.load(key).map(|(_, state)| state)
    }

    /// Moves the local ticket of any run to the status the run implies, as the flow actor.
    /// A missing note or an illegal edge leaves the ticket as it is.
    fn sync_ticket_status(&self, state: &FlowState) {
        let Some(target) = ticket_status_of(state) else {
            return;
        };
        let vault = self.store.vault_root();
        if !vault.join(TICKETS_DIR).is_dir() {
            return;
        }
        if let Err(error) = self.write_ticket_status(vault, state, target) {
            tracing::debug!(key = %state.key, %error, "ticket status left unchanged");
        }
    }

    fn write_ticket_status(
        &self,
        vault: &std::path::Path,
        state: &FlowState,
        target: Status,
    ) -> anyhow::Result<()> {
        let tickets = TicketStore::open(vault)?;
        let ticket_key = pipeline_inputs::ticket_key(state);
        let (mut ticket, body, extra) = tickets.get(ticket_key)?;
        let from = ticket.status;
        let flow_changed = ticket.autopilot.flow.as_deref() != Some(state.def.id.as_str());
        if from == target && !flow_changed {
            return Ok(());
        }
        let moves = from != target;
        if moves {
            guard(from, target, Actor::Flow, false)?;
            ticket.status = target;
        }
        if target == Status::Blocked {
            let refused = state.steps.values().any(|step| {
                step.status == StepStatus::Escalated
                    && step.reason.as_deref() == Some(StepReason::TicketBudget.as_str())
            });
            let reason = if refused {
                FlowTicketReason::Budget
            } else {
                FlowTicketReason::Escalated
            };
            ticket.autopilot.blocked_reason = Some(reason.as_str().to_string());
        }
        ticket.autopilot.flow = Some(state.def.id.clone());
        tickets.save(&mut ticket, &body, &extra)?;
        if moves {
            ledger::append(
                vault,
                &ledger::Event {
                    ts: chrono::Utc::now(),
                    key: ticket_key.to_string(),
                    from: Some(from),
                    to: target,
                    actor: Actor::Flow,
                    reason: Some(FlowTicketReason::StatusSync.as_str().to_string()),
                    session_id: None,
                },
            );
        }
        Ok(())
    }

    fn apply_effects(
        self: &Arc<Self>,
        repo: &str,
        key: &str,
        effects: Vec<Effect>,
    ) -> anyhow::Result<()> {
        let mut light_ids: Vec<String> = Vec::new();
        for effect in effects {
            let state = self.load(key)?.1;
            if state.effects_applied.contains(&effect.id) {
                continue;
            }
            if let Err(error) = self.apply_effect(repo, &state, &effect) {
                tracing::warn!(key, effect = %effect.id, %error, "flow effect failed");
            }
            if matches!(
                effect.kind,
                EffectKind::Journal { .. } | EffectKind::Emit { .. }
            ) {
                light_ids.push(effect.id);
            } else {
                self.mark_applied(repo, key, &[effect.id])?;
            }
        }
        if light_ids.is_empty() {
            return Ok(());
        }
        self.mark_applied(repo, key, &light_ids)
    }

    fn mark_applied(&self, repo: &str, key: &str, ids: &[String]) -> anyhow::Result<()> {
        let (_, mut state) = self.load(key)?;
        state.effects_applied.extend(ids.iter().cloned());
        self.store.save_state(repo, &state, &self.now())
    }

    fn apply_effect(
        self: &Arc<Self>,
        repo: &str,
        state: &FlowState,
        effect: &Effect,
    ) -> anyhow::Result<()> {
        let key = state.key.as_str();
        match &effect.kind {
            EffectKind::Journal { at, line } => {
                self.store.append_journal(
                    repo,
                    key,
                    &JournalLine {
                        at: at.clone(),
                        line: line.clone(),
                        actor: None,
                        id: Some(effect.id.clone()),
                    },
                );
            }
            EffectKind::Emit {
                event,
                step,
                detail,
            } => {
                events::publish(
                    &self.ports.bus,
                    *event,
                    state,
                    step.as_deref(),
                    detail.as_deref(),
                );
            }
            EffectKind::RunAction { step, name, with } => {
                if step.starts_with(MANUAL_PREFIX) && self.external_action(repo, key, name) {
                    let entry = ExternalManualEntry {
                        step: step.clone(),
                        name: name.clone(),
                        with: with.clone(),
                        created_at: self.now(),
                    };
                    self.hold_manual(repo, key, &effect.id, entry)?;
                } else {
                    self.spawn_action(ActionJob {
                        key: key.to_string(),
                        repo: repo.to_string(),
                        effect_id: effect.id.clone(),
                        name: name.clone(),
                        with: with.clone(),
                    });
                }
            }
            EffectKind::StartPipeline {
                step,
                template,
                with,
            } => self.start_pipeline(repo, state, &effect.id, step, template, with)?,
            EffectKind::CancelSession { step } => self.cancel_session(repo, state, step)?,
            EffectKind::CreateInboxEntry {
                step,
                prompt,
                choices,
            } => self.create_inbox(repo, state, step, prompt.clone(), choices.clone())?,
            EffectKind::ResolveInboxEntry { step } => self.resolve_inbox(repo, key, step)?,
            EffectKind::StartWait { step, probe, with } => {
                self.start_wait(repo, state, &effect.id, step, probe, with)?
            }
        }
        Ok(())
    }

    /// Adds a line to the run's journal, attributed to `actor` when given.
    pub fn journal(&self, key: &str, actor: Option<&str>, line: String) {
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Ok((repo, _)) = self.load(key) else {
            return;
        };
        self.store.append_journal(
            &repo,
            key,
            &JournalLine {
                at: self.now(),
                line,
                actor: actor.map(str::to_string),
                id: None,
            },
        );
    }

    fn spawn_action(self: &Arc<Self>, job: ActionJob) {
        if self.external_action(&job.repo, &job.key, &job.name) {
            return;
        }
        let _admission = lock(&self.action_admission);
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        if !lock(&self.inflight).insert(job.inflight_id()) {
            return;
        }
        let executor = self.clone();
        let worker = std::thread::spawn(move || {
            threads::guarded(FlowWorker::Action.as_str(), || executor.run_job(&job));
            lock(&executor.inflight).remove(&job.inflight_id());
        });
        let mut workers = lock(&self.action_workers);
        let mut running = Vec::new();
        for completed in workers.drain(..) {
            if completed.is_finished() {
                let _ = completed.join();
            } else {
                running.push(completed);
            }
        }
        running.push(worker);
        *workers = running;
    }

    fn run_job(self: &Arc<Self>, job: &ActionJob) {
        let outcome = match self.project_gate(job) {
            Some(held) => held,
            None => {
                let Some(outcome) = self.execute_action(job) else {
                    return;
                };
                self.project_job_done(job, &outcome);
                outcome
            }
        };
        if job.name == crate::models::constants::flow_actions::ACTION_GH_MERGE
            && matches!(outcome, ActionOutcome::Ok(_))
        {
            self.ports.bus.emit(
                crate::memory_ingest::EVENT_FLOW_MERGED,
                serde_json::json!({ "key": job.key, "repo": job.repo }),
            );
        }
        self.deliver_quiet(
            &job.key,
            Event::ActionFinished {
                effect_id: job.effect_id.clone(),
                outcome,
                at: self.now(),
            },
        );
    }

    /// Runs the action. `None` means the workspace stayed busy: the step
    /// stays running and the next reconcile launches the action again.
    fn execute_action(&self, job: &ActionJob) -> Option<ActionOutcome> {
        if self.stopping.load(Ordering::SeqCst) {
            return None;
        }
        let Some(action) = self.registry.action(&job.name) else {
            return Some(ActionOutcome::TechnicalError(format!(
                "{UNKNOWN_ACTION_PREFIX} `{}`",
                job.name
            )));
        };
        let vault = self.store.vault_root();
        let workspace_id = workspace_key(vault, &job.repo, &job.key);
        let _guard = if action.writes() {
            Some(self.acquire(&workspace_id, job)?)
        } else {
            None
        };
        let project_dir = self
            .load(&job.key)
            .ok()
            .map(|(_, state)| state)
            .filter(project_checkout::is_project_flow)
            .and_then(|state| self.project_checkout_path(&state));
        let workspace = project_dir
            .unwrap_or_else(|| crate::worktrees::worktree_path(vault, &job.repo, &job.key));
        let action_key = job.key.as_str();
        let context = ActionContext {
            key: action_key,
            workspace: workspace.is_dir().then_some(workspace.as_path()),
            runner: &self.action_processes,
        };
        let outcome = crate::threads::guarded(FLOWS_ACTION_LOOP, || {
            let with = self.with_facts(job, action.placeholders(), true);
            action.run(&context, &with)
        });
        Some(outcome.unwrap_or_else(|| ActionOutcome::TechnicalError(PANIC_MESSAGE.to_string())))
    }

    /// The job's `with`, completed with the run's facts for every placeholder it leaves out.
    /// `live` allows the one `gh` read behind `sha`; the external listing never makes it.
    pub(super) fn with_facts(
        &self,
        job: &ActionJob,
        placeholders: Vec<String>,
        live: bool,
    ) -> BTreeMap<String, Value> {
        let mut with = job.with.clone();
        let missing: Vec<String> = placeholders
            .into_iter()
            .filter(|name| !with.contains_key(name))
            .collect();
        if missing.is_empty() {
            return with;
        }
        let Ok((_, state)) = self.load(&job.key) else {
            return with;
        };
        let source = self.registry.pr_status_source();
        let status = || {
            live.then(|| facts::status_within(source, &job.key, LIVE_STATUS_TIMEOUT))
                .flatten()
        };
        for (name, value) in facts::collect(&state, &missing, status) {
            with.entry(name).or_insert(value);
        }
        with
    }

    fn acquire(&self, workspace_id: &str, job: &ActionJob) -> Option<WorkspaceGuard> {
        let deadline = Instant::now() + self.ports.lock_wait;
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                return None;
            }
            if let Some(guard) =
                WorkspaceGuard::acquire_for_stage(&self.ports.lock, workspace_id, &job.name)
            {
                return Some(self.record_holder(guard, job));
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(LOCK_POLL);
        }
    }

    /// Records the action as holder of the PR branch in the vault, removed when the guard drops.
    fn record_holder(&self, guard: WorkspaceGuard, job: &ActionJob) -> WorkspaceGuard {
        let Some(number) = pr_workspace_holder::pr_number_of(&job.with) else {
            return guard;
        };
        let vault = self.store.vault_root().to_path_buf();
        let holder = PrWorkspaceHolder {
            stage: job.name.clone(),
            run_id: None,
            session_id: None,
            acquired_at: now_iso_with(self.ports.clock.as_ref()),
        };
        if let Err(error) = pr_workspace_holder::write(&vault, &job.repo, number, &holder) {
            tracing::warn!(%error, number, "workspace holder not recorded");
            return guard;
        }
        let repo = job.repo.clone();
        guard.on_release(move || pr_workspace_holder::remove(&vault, &repo, number))
    }

    fn start_pipeline(
        self: &Arc<Self>,
        repo: &str,
        state: &FlowState,
        effect: &str,
        step: &str,
        template: &str,
        with: &BTreeMap<String, Value>,
    ) -> anyhow::Result<()> {
        let key = state.key.as_str();
        let policy = self
            .store
            .load_runtime(repo, key)
            .start_request
            .and_then(|request| request.policy);
        if !self.ports.budget.allows(template) {
            if step.starts_with(MANUAL_PREFIX) {
                self.journal(
                    key,
                    None,
                    format!("{step} not started: {}", StepReason::Budget.as_str()),
                );
            } else {
                self.deliver_locked(
                    key,
                    Event::BudgetInsufficient {
                        step: step.to_string(),
                        at: self.now(),
                    },
                )?;
            }
            return Ok(());
        }
        if !step.starts_with(MANUAL_PREFIX) && self.hold_for_ticket_ceiling(key, state, step)? {
            return Ok(());
        }
        let Some(project) = self.ports.sessions.project_for(&state.ticket) else {
            return self.pipeline_start_failed(key, effect, NO_PROJECT_MESSAGE.to_string());
        };
        if let Some(name) = self.wait_for_resource(state, step) {
            return self
                .deliver_locked(
                    key,
                    Event::ResourceBusy {
                        step: step.to_string(),
                        resource: name,
                        at: self.now(),
                    },
                )
                .map(|_| ());
        }
        if let Some(failure) = self.reenter_project_checkout(repo, state) {
            let reason = match failure {
                ActionOutcome::TechnicalError(text) | ActionOutcome::BusinessError(text) => text,
                _ => NO_CHECKOUT_MESSAGE.to_string(),
            };
            return self.pipeline_start_failed(key, effect, reason);
        }
        let iteration = state.steps.get(step).map_or(0, |s| s.iteration);
        if self.resume_previous_run(repo, key, effect, step, iteration)? {
            return Ok(());
        }
        let inputs_file = self.write_inputs(repo, state, step, iteration, with)?;
        let launch = PipelineLaunch {
            key: key.to_string(),
            ticket_key: pipeline_inputs::ticket_key(state).to_string(),
            provider: None,
            permission_mode: None,
            strict_mcp_config: None,
            mcp_config_id: None,
            mcp_servers: None,
            denied_shell_commands: Vec::new(),
            denied_mcp_tools: BTreeMap::new(),
            step: step.to_string(),
            template: template.to_string(),
            project,
            inputs_file,
            auto_approve_gates: gates_of(with),
            autonomous: with
                .get(WITH_AUTONOMOUS)
                .and_then(Value::as_bool)
                .unwrap_or(DEFAULT_AUTONOMOUS),
            headless: false,
            engine: with
                .get(WITH_ENGINE)
                .and_then(Value::as_str)
                .and_then(Engine::parse),
            agent_executor: AgentExecutor::Internal,
            project_checkout: project_checkout::is_project_flow(state),
        };
        let mut launch = launch;
        if let Some(policy) = policy {
            launch.autonomous = policy.autonomous;
            launch.auto_approve_gates = Some(policy.auto_approve_gates);
            launch.provider = policy.provider;
            launch.permission_mode = policy.permission_mode;
            launch.strict_mcp_config = policy.strict_mcp_config;
            launch.mcp_config_id = policy.mcp_config_id;
            launch.mcp_servers = policy.mcp_servers;
            launch.denied_shell_commands = policy.denied_shell_commands;
            launch.denied_mcp_tools = policy.denied_mcp_tools;
            launch.headless = policy.headless;
            if policy.external_agents {
                launch.agent_executor = AgentExecutor::External;
            }
        }
        match self.ports.sessions.start(&launch) {
            Ok(SessionStart::Started(session_id)) => {
                let mut runtime = self.store.load_runtime(repo, key);
                runtime.sessions.insert(effect.to_string(), session_id);
                self.store.save_runtime(repo, key, &runtime)
            }
            Ok(SessionStart::Busy) => {
                if step.starts_with(MANUAL_PREFIX) {
                    self.journal(key, None, format!("{step} not started: {WORKSPACE_BUSY}"));
                }
                Ok(())
            }
            Err(error) => self.pipeline_start_failed(key, effect, error.to_string()),
        }
    }

    /// A retried pipeline step continues the previous iteration's run when that run is paused,
    /// instead of starting the pipeline again from its first stage.
    fn resume_previous_run(
        &self,
        repo: &str,
        key: &str,
        effect: &str,
        step: &str,
        iteration: u32,
    ) -> anyhow::Result<bool> {
        if iteration == 0 || step.starts_with(MANUAL_PREFIX) {
            return Ok(false);
        }
        let previous = effect_id(step, iteration - 1, EffectName::StartPipeline.as_str());
        let mut runtime = self.store.load_runtime(repo, key);
        let Some(run_id) = runtime.sessions.get(&previous).cloned() else {
            return Ok(false);
        };
        if !self.ports.sessions.resume_paused(&run_id) {
            return Ok(false);
        }
        runtime.sessions.insert(effect.to_string(), run_id.clone());
        self.store.save_runtime(repo, key, &runtime)?;
        self.journal(key, None, format!("{step} resumed paused run {run_id}"));
        Ok(true)
    }

    fn pipeline_start_failed(
        self: &Arc<Self>,
        key: &str,
        effect: &str,
        reason: String,
    ) -> anyhow::Result<()> {
        self.journal(key, None, format!("pipeline could not start: {reason}"));
        self.deliver_locked(
            key,
            Event::PipelineFinished {
                effect_id: effect.to_string(),
                run_status: PipelineRunStatus::Failed.as_str().to_string(),
                outputs: BTreeMap::new(),
                at: self.now(),
            },
        )
        .map(|_| ())
    }

    fn write_inputs(
        &self,
        repo: &str,
        state: &FlowState,
        step: &str,
        iteration: u32,
        with: &BTreeMap<String, Value>,
    ) -> anyhow::Result<PathBuf> {
        let body = self.ticket_body(pipeline_inputs::ticket_key(state));
        let report = pipeline_inputs::report_path(state, with)
            .and_then(|path| std::fs::read_to_string(path).ok());
        let inputs = pipeline_inputs::build(state, body.as_deref(), report.as_deref(), with);
        let dir = self.store.inputs_dir(repo, &state.key);
        std::fs::create_dir_all(&dir)?;
        let launch_id = self.ports.ids.new_run_id(self.ports.clock.as_ref());
        let file = dir.join(format!("{}-{iteration}-{launch_id}.json", file_safe(step)));
        std::fs::write(&file, ascii_json(&Value::Object(inputs)))?;
        Ok(file)
    }

    /// The note body of ticket `key`, the demand of a local ticket; `None` when the vault has no such note.
    fn ticket_body(&self, key: &str) -> Option<String> {
        let tickets = TicketStore::open(self.store.vault_root()).ok()?;
        tickets.get(key).ok().map(|(_, body, _)| body)
    }

    fn cancel_session(&self, repo: &str, state: &FlowState, step: &str) -> anyhow::Result<()> {
        let iteration = state.steps.get(step).map_or(0, |s| s.iteration);
        let effect = effect_id(step, iteration, EffectName::StartPipeline.as_str());
        let mut runtime = self.store.load_runtime(repo, &state.key);
        if let Some(session_id) = runtime.sessions.remove(&effect) {
            self.ports.sessions.cancel(&session_id);
            self.store.save_runtime(repo, &state.key, &runtime)?;
        }
        Ok(())
    }

    /// Cancels the sessions of manual pipelines of run `key`.
    pub fn cancel_manual(&self, key: &str) {
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Ok((repo, _)) = self.load(key) else {
            return;
        };
        let mut runtime = self.store.load_runtime(&repo, key);
        let manual: Vec<String> = runtime
            .sessions
            .keys()
            .filter(|effect| effect.starts_with(MANUAL_PREFIX))
            .cloned()
            .collect();
        for effect in manual {
            if let Some(session_id) = runtime.sessions.remove(&effect) {
                self.ports.sessions.cancel(&session_id);
            }
        }
        let _ = self.store.save_runtime(&repo, key, &runtime);
    }

    /// Asks "continue?" instead of starting a pipeline step for a ticket past its token ceiling.
    fn hold_for_ticket_ceiling(
        self: &Arc<Self>,
        key: &str,
        state: &FlowState,
        step: &str,
    ) -> anyhow::Result<bool> {
        let ticket = pipeline_inputs::ticket_key(state);
        let Some(hit) = self.ports.budget.ticket_ceiling(ticket) else {
            return Ok(false);
        };
        self.ports.bus.emit(
            TICKET_BUDGET_EVENT,
            serde_json::json!({
                "key": key,
                "ticket": ticket,
                "step": step,
                "what": hit.what,
                "used": hit.used,
                "limit": hit.limit,
            }),
        );
        self.deliver_locked(
            key,
            Event::TicketBudgetExceeded {
                step: step.to_string(),
                detail: hit.prompt,
                at: self.now(),
            },
        )?;
        Ok(true)
    }

    /// An approved "continue?" lifts the ceilings of the ticket before the step starts again.
    fn raise_ceiling_on_approval(&self, state: &FlowState, event: &Event) {
        let Event::HumanDecision { step, decision, .. } = event else {
            return;
        };
        let held = state.steps.get(step).is_some_and(|held| {
            held.status == StepStatus::WaitingHuman
                && held.reason.as_deref() == Some(StepReason::TicketBudget.as_str())
        });
        let approved = HumanDecision::parse(decision).is_some_and(HumanDecision::is_ok);
        if held && approved {
            self.ports
                .budget
                .raise_ticket_ceiling(pipeline_inputs::ticket_key(state));
        }
    }

    fn create_inbox(
        &self,
        repo: &str,
        state: &FlowState,
        step: &str,
        prompt: Option<String>,
        choices: Vec<String>,
    ) -> anyhow::Result<()> {
        let iteration = state.steps.get(step).map_or(0, |s| s.iteration);
        let mut runtime = self.store.load_runtime(repo, &state.key);
        let known = runtime
            .inbox
            .iter()
            .any(|item| item.step == step && item.iteration == iteration);
        if known {
            return Ok(());
        }
        let id = [&state.key, step, &iteration.to_string()].join(ID_SEPARATOR);
        runtime.inbox.push(InboxItem {
            id,
            step: step.to_string(),
            iteration,
            prompt,
            choices,
            created_at: self.now(),
            resolved_at: None,
        });
        self.store.save_runtime(repo, &state.key, &runtime)
    }

    fn resolve_inbox(&self, repo: &str, key: &str, step: &str) -> anyhow::Result<()> {
        let mut runtime = self.store.load_runtime(repo, key);
        let now = self.now();
        for item in runtime
            .inbox
            .iter_mut()
            .filter(|item| item.step == step && item.resolved_at.is_none())
        {
            item.resolved_at = Some(now.clone());
        }
        self.store.save_runtime(repo, key, &runtime)
    }

    fn start_wait(
        self: &Arc<Self>,
        repo: &str,
        state: &FlowState,
        effect: &str,
        step: &str,
        probe: &str,
        with: &BTreeMap<String, Value>,
    ) -> anyhow::Result<()> {
        let mut runtime = self.store.load_runtime(repo, &state.key);
        let record = WaitRecord {
            step: step.to_string(),
            probe: probe.to_string(),
            with: with.clone(),
            started_ms: self.ports.clock.now_ms(),
            state: None,
        };
        runtime.waits.insert(effect.to_string(), record.clone());
        self.store.save_runtime(repo, &state.key, &runtime)?;
        self.poll_wait(repo, &state.key, effect, &record)
    }

    /// Asks the probe whether its fact holds; delivers `WaitSatisfied` when it does.
    fn poll_wait(
        self: &Arc<Self>,
        repo: &str,
        key: &str,
        effect: &str,
        record: &WaitRecord,
    ) -> anyhow::Result<()> {
        if self.external_probe(repo, key, &record.probe) {
            return Ok(());
        }
        let Some(probe) = self.registry.probe(&record.probe) else {
            tracing::warn!(key, probe = %record.probe, "unknown wait probe");
            return Ok(());
        };
        let context = ProbeContext {
            key: key.to_string(),
            now_ms: self.ports.clock.now_ms(),
            started_ms: record.started_ms,
            last_state: record.state.clone(),
            pr_status: self.registry.pr_status_source(),
        };
        let outcome = probe.poll(&context, &record.with);
        let mut runtime = self.store.load_runtime(repo, key);
        if !outcome.satisfied {
            self.remember_probe_state(&mut runtime, repo, key, effect, outcome.state)?;
            return Ok(());
        }
        runtime.waits.remove(effect);
        self.store.save_runtime(repo, key, &runtime)?;
        self.deliver_locked(
            key,
            Event::WaitSatisfied {
                effect_id: effect.to_string(),
                outputs: outcome.outputs,
                at: self.now(),
            },
        )
        .map(|_| ())
    }

    /// Stores the probe's state in its wait record when it changed.
    pub(super) fn remember_probe_state(
        &self,
        runtime: &mut Runtime,
        repo: &str,
        key: &str,
        effect: &str,
        state: Option<Value>,
    ) -> anyhow::Result<()> {
        let Some(stored) = runtime.waits.get_mut(effect) else {
            return Ok(());
        };
        if stored.state == state {
            return Ok(());
        }
        stored.state = state;
        self.store.save_runtime(repo, key, runtime)
    }

    /// Re-checks every run that can still change.
    pub fn reconcile_all(self: &Arc<Self>) -> usize {
        if self.stopping.load(Ordering::SeqCst) {
            return 0;
        }
        self.restore_resources();
        let active: Vec<String> = self
            .store
            .index()
            .into_values()
            .filter(|entry| is_active(entry.status))
            .map(|entry| entry.key)
            .collect();
        for key in &active {
            if let Err(error) = self.reconcile(key) {
                tracing::warn!(key = %key, %error, "flow reconcile failed");
            }
        }
        active.len()
    }

    /// The resources of the daemon (the Unity editor lock).
    pub fn resources(&self) -> &ResourceManager {
        &self.resources
    }

    /// The resource key (`unity-editor:<project id>`) and name a step of `state` needs.
    fn resource_of(&self, state: &FlowState, step: &Step) -> Option<(String, String)> {
        let name = step.resource.clone().or_else(|| {
            (project_checkout::is_project_flow(state) && step.kind == StepKind::Pipeline)
                .then(|| resource::UNITY_EDITOR.to_string())
        })?;
        let project = self.ports.sessions.project_for(&state.ticket)?;
        Some((unity_lock::resource_key(&name, &project.id), name))
    }

    /// What run `state` does with resources: the key it holds (its pipeline step runs) and the
    /// key it waits for.
    fn resource_role(&self, state: &FlowState) -> (Option<String>, Option<String>) {
        let none = (None, None);
        if project_checkout::is_project_flow(state) {
            return self.project_role(state);
        }
        let Some(rid) = state.current.as_ref() else {
            return none;
        };
        let (Some(step_state), Some(step)) = (state.steps.get(rid), step_at_cursor(state)) else {
            return none;
        };
        if step.kind != StepKind::Pipeline || !rid.ends_with(&step.id) {
            return none;
        }
        let Some((key, _)) = self.resource_of(state, &step) else {
            return none;
        };
        match step_state.status {
            StepStatus::Running => (Some(key), None),
            StepStatus::WaitingExternal
                if step_state
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.starts_with(resource::REASON_PREFIX)) =>
            {
                (None, Some(key))
            }
            _ => none,
        }
    }

    fn resource_holder(&self, state: &FlowState) -> ResourceHolder {
        ResourceHolder {
            flow_key: state.key.clone(),
            ticket_key: pipeline_inputs::ticket_key(state).to_string(),
            step_id: state.current.clone().unwrap_or_default(),
            since: self.now(),
        }
    }

    /// Takes the resource of the step about to start. `Some(name)` when it is held by another
    /// run: the caller parks the step.
    fn wait_for_resource(self: &Arc<Self>, state: &FlowState, step: &str) -> Option<String> {
        if step.starts_with(MANUAL_PREFIX) || state.current.as_deref() != Some(step) {
            return None;
        }
        let definition = step_at_cursor(state)?;
        let (resource, name) = self.resource_of(state, &definition)?;
        let (answer, change) = self
            .resources
            .acquire(&resource, self.resource_holder(state));
        self.announce(change.into_iter().collect());
        (answer == Acquire::Busy).then_some(name)
    }

    /// Frees what run `state` no longer holds or waits for.
    fn sync_resources(self: &Arc<Self>, state: &FlowState) {
        let (holds, waiting) = self.resource_role(state);
        let changes = self
            .resources
            .sync(&state.key, holds.as_deref(), waiting.as_deref());
        self.announce(changes);
    }

    /// Rebuilds the holders from the persisted runs: a running pipeline step with a resource holds it.
    fn restore_resources(self: &Arc<Self>) {
        for entry in self
            .store
            .index()
            .into_values()
            .filter(|entry| is_active(entry.status))
        {
            let Ok((_, state)) = self.load(&entry.key) else {
                continue;
            };
            if let (Some(resource), _) = self.resource_role(&state) {
                let (_, change) = self
                    .resources
                    .claim(&resource, self.resource_holder(&state));
                self.announce(change.into_iter().collect());
            }
        }
    }

    /// Publishes `unity.lock_changed` for each change and wakes the waiters of freed resources.
    fn announce(self: &Arc<Self>, changes: Vec<Change>) {
        let mut wake: Vec<String> = Vec::new();
        for change in changes {
            self.ports.bus.emit(
                unity_lock::EVENT_LOCK_CHANGED,
                serde_json::to_value(&change).unwrap_or_default(),
            );
            if change.released {
                wake.extend(change.waiting);
            }
        }
        if wake.is_empty() || self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let executor = self.clone();
        let worker = std::thread::spawn(move || {
            for key in wake {
                if let Err(error) = executor.reconcile(&key) {
                    tracing::debug!(key = %key, %error, "waiter not woken");
                }
            }
        });
        let mut workers = lock(&self.action_workers);
        workers.retain(|worker| !worker.is_finished());
        workers.push(worker);
    }

    /// Frees a resource whoever holds it, journals it on the holder's run and publishes the change.
    /// `None` when nobody held it.
    pub fn release_resource(self: &Arc<Self>, resource: &str) -> Option<ResourceHolder> {
        let (holder, change) = self.resources.force_release(resource)?;
        self.journal(
            &holder.flow_key,
            Some("api"),
            format!("resource {resource} released by hand"),
        );
        self.announce(vec![change]);
        Some(holder)
    }

    /// Idempotent resync: sessions, waits, budget, lost effects.
    pub fn reconcile(self: &Arc<Self>, key: &str) -> anyhow::Result<()> {
        let _transition = self.transition()?;
        let guard = self.key_lock(key);
        let _held = lock(&guard);
        let (repo, state) = self.load(key)?;
        self.reconcile_manual(&repo, key)?;
        let Some(step_id) = state.current.clone() else {
            return Ok(());
        };
        let (Some(step_state), Some(step)) = (state.steps.get(&step_id), step_at_cursor(&state))
        else {
            return Ok(());
        };
        let iteration = step_state.iteration;
        match (step.kind, step_state.status) {
            (StepKind::Pipeline, StepStatus::Running) => {
                self.reconcile_pipeline(&repo, &state, &step_id, iteration, &step)
            }
            (StepKind::Pipeline, StepStatus::WaitingExternal)
                if step_state.reason.as_deref() == Some(StepReason::Budget.as_str()) =>
            {
                self.retry_when_budget_allows(key, &step_id, &step)
            }
            (StepKind::Pipeline, StepStatus::WaitingExternal)
                if step_state
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.starts_with(resource::REASON_PREFIX)) =>
            {
                self.retry_when_resource_free(&state, &step_id, &step)
            }
            (StepKind::Action, StepStatus::WaitingExternal)
                if project_checkout::waits_for_project(&state) =>
            {
                self.retry_project_when_free(&state, &step_id)
            }
            (StepKind::Pipeline, StepStatus::WaitingHuman)
                if step_state.reason.as_deref() == Some(StepReason::TicketBudget.as_str()) =>
            {
                self.create_inbox(
                    &repo,
                    &state,
                    &step_id,
                    step_state.detail.clone(),
                    vec![
                        HumanDecision::Approve.as_str().to_string(),
                        HumanDecision::Reject.as_str().to_string(),
                    ],
                )
            }
            (StepKind::Action, StepStatus::Running) => {
                let effect = effect_id(&step_id, iteration, EffectName::RunAction.as_str());
                self.spawn_action(ActionJob {
                    key: key.to_string(),
                    repo,
                    effect_id: effect,
                    name: step.use_.clone().unwrap_or_default(),
                    with: step.with.clone(),
                });
                Ok(())
            }
            (StepKind::Human, StepStatus::WaitingHuman) => self.create_inbox(
                &repo,
                &state,
                &step_id,
                step.prompt.clone(),
                step.choices.clone(),
            ),
            (StepKind::Wait, StepStatus::WaitingExternal) => {
                self.reconcile_wait(&repo, &state, &step_id, iteration, &step)
            }
            _ => Ok(()),
        }
    }

    fn reconcile_pipeline(
        self: &Arc<Self>,
        repo: &str,
        state: &FlowState,
        step_id: &str,
        iteration: u32,
        step: &Step,
    ) -> anyhow::Result<()> {
        let key = state.key.as_str();
        let effect = effect_id(step_id, iteration, EffectName::StartPipeline.as_str());
        let runtime = self.store.load_runtime(repo, key);
        let Some(session_id) = runtime.sessions.get(&effect) else {
            let template = step.use_.clone().unwrap_or_default();
            return self.start_pipeline(repo, state, &effect, step_id, &template, &step.with);
        };
        let event = match self.ports.sessions.state(session_id) {
            SessionState::Running => return Ok(()),
            SessionState::Ended {
                run_status,
                outputs,
            } => Event::PipelineFinished {
                effect_id: effect,
                run_status,
                outputs,
                at: self.now(),
            },
            SessionState::Gone => Event::PipelineOrphaned {
                effect_id: effect,
                at: self.now(),
            },
        };
        self.deliver_locked(key, event).map(|_| ())
    }

    /// Retries a step waiting for a resource once the resource can go to it.
    fn retry_when_resource_free(
        self: &Arc<Self>,
        state: &FlowState,
        step_id: &str,
        step: &Step,
    ) -> anyhow::Result<()> {
        if let Some((resource, _)) = self.resource_of(state, step) {
            if !self.resources.can_grant(&resource, &state.key) {
                return Ok(());
            }
        }
        self.deliver_locked(
            &state.key,
            Event::Retry {
                step: step_id.to_string(),
                at: self.now(),
            },
        )
        .map(|_| ())
    }

    fn retry_project_when_free(
        self: &Arc<Self>,
        state: &FlowState,
        step_id: &str,
    ) -> anyhow::Result<()> {
        if let Some(resource) = self.project_resource(state) {
            if !self.resources.can_grant(&resource, &state.key) {
                return Ok(());
            }
        }
        self.deliver_locked(
            &state.key,
            Event::Retry {
                step: step_id.to_string(),
                at: self.now(),
            },
        )
        .map(|_| ())
    }

    fn retry_when_budget_allows(
        self: &Arc<Self>,
        key: &str,
        step_id: &str,
        step: &Step,
    ) -> anyhow::Result<()> {
        let template = step.use_.clone().unwrap_or_default();
        if !self.ports.budget.allows(&template) {
            return Ok(());
        }
        self.deliver_locked(
            key,
            Event::Retry {
                step: step_id.to_string(),
                at: self.now(),
            },
        )
        .map(|_| ())
    }

    fn reconcile_wait(
        self: &Arc<Self>,
        repo: &str,
        state: &FlowState,
        step_id: &str,
        iteration: u32,
        step: &Step,
    ) -> anyhow::Result<()> {
        let key = state.key.as_str();
        let effect = effect_id(step_id, iteration, EffectName::StartWait.as_str());
        let runtime = self.store.load_runtime(repo, key);
        match runtime.waits.get(&effect) {
            Some(record) => self.poll_wait(repo, key, &effect, record),
            None => {
                let probe = step.use_.clone().unwrap_or_default();
                self.start_wait(repo, state, &effect, step_id, &probe, &step.with)
            }
        }
    }

    /// Reports the end of sessions started by manual requests, then forgets them.
    fn reconcile_manual(self: &Arc<Self>, repo: &str, key: &str) -> anyhow::Result<()> {
        let runtime: Runtime = self.store.load_runtime(repo, key);
        let manual: Vec<(String, String)> = runtime
            .sessions
            .iter()
            .filter(|(effect, _)| effect.starts_with(MANUAL_PREFIX))
            .map(|(effect, session)| (effect.clone(), session.clone()))
            .collect();
        for (effect, session_id) in manual {
            let (run_status, outputs) = match self.ports.sessions.state(&session_id) {
                SessionState::Running => continue,
                SessionState::Ended {
                    run_status,
                    outputs,
                } => (run_status, outputs),
                SessionState::Gone => (
                    PipelineRunStatus::Orphaned.as_str().to_string(),
                    BTreeMap::new(),
                ),
            };
            let outcome = self.deliver_locked(
                key,
                Event::PipelineFinished {
                    effect_id: effect.clone(),
                    run_status,
                    outputs,
                    at: self.now(),
                },
            );
            if let Err(error) = outcome {
                tracing::debug!(key, %error, "manual pipeline result dropped");
            }
            let mut fresh = self.store.load_runtime(repo, key);
            fresh.sessions.remove(&effect);
            self.store.save_runtime(repo, key, &fresh)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/executor_tests.rs"]
mod tests;
