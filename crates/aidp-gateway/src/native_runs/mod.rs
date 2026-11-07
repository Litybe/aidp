//! Native runs: the daemon executes a v7 run itself, one headless session per stage,
//! following the execution plan compiled by aidp-core.

pub mod agent_board;
pub mod agent_job_builder;
pub mod agent_verdict;
pub mod autonomy;
pub mod dispatch;
pub mod exec_loop;
pub mod external_session;
pub mod http_routes;
pub mod recovery;
pub mod reply;
pub mod scope_guard;
pub mod state;
pub mod task_isolation;

use crate::bus::Bus;
use crate::flows::lock::WorkspaceLock;
use crate::flows::registry::BudgetGuard;
use crate::inbox::InboxEntry;
use crate::models::agent_claim::AgentClaim;
use crate::models::agent_claim_request::AgentClaimRequest;
use crate::models::agent_heartbeat::AgentHeartbeat;
use crate::models::agent_heartbeat_request::AgentHeartbeatRequest;
use crate::models::agent_job_ack::AgentJobAck;
use crate::models::agent_job_result::AgentJobResult;
use crate::models::agent_job_summary::AgentJobSummary;
use crate::models::constants::agent_jobs::{ERROR_FOREIGN_JOB, ERROR_NOT_EXTERNAL};
use crate::models::constants::flow_inputs::INPUT_PROMPT;
use crate::models::constants::pr_workspace_holder::HOLDER_THREAD_SUFFIX;
use crate::models::enums::agent_executor::AgentExecutor;
use crate::models::enums::native_reason::NativeReason;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::native_stage_status::NativeStageStatus;
use crate::models::enums::{InboxAction, InboxKind};
use crate::models::native_run_start::StartNativeRun;
use crate::models::native_run_view::NativeRunView;
use crate::models::pr_workspace_holder::PrWorkspaceHolder;
use crate::pr_workspace_holder;
use crate::projects::Projects;
use crate::session_error::SessionError;
use crate::sessions::SessionManager;
use crate::sync::lock;
use crate::threads;
use aidp_core::clock::SystemClock;
use aidp_core::cost::PriceTable;
use aidp_core::git::GitCli;
use aidp_core::id::SystemIdGen;
use aidp_core::models::run_state::RunPorts;
use aidp_core::pipeline::{self, Pipeline};
use aidp_core::run::{self, INPUT_JIRA_KEY, INPUT_TARGET};
use aidp_core::vault::Vault;
use exec_loop::{run_lock_key, RunExec};
use state::{RunControl, RunStore};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Bus event published when a run begins executing.
pub const EVENT_RUN_STARTED: &str = "run.started";
/// Bus event published when a stage, task or shard changes state, and when a run pauses.
pub const EVENT_RUN_STAGE_CHANGED: &str = "run.stage_changed";
/// Bus event published when a run stops at a gate.
pub const EVENT_RUN_WAITING_GATE: &str = "run.waiting_gate";
/// Bus event published when a run is done, failed or cancelled.
pub const EVENT_RUN_ENDED: &str = "run.ended";

const TEMPLATES_DIR: &str = "templates";
const LOCAL_TEMPLATES_DIR: &str = ".aidp/pipeline/flow";
const THREAD_PREFIX: &str = "native-run-";
const THREAD_NAME: &str = "native-run";
const DEFAULT_POLL: Duration = Duration::from_millis(200);
const LOCK_RETRY: Duration = Duration::from_millis(20);
const LOCK_WAIT: Duration = Duration::from_secs(2);
const WAIT_POLL: Duration = Duration::from_millis(50);
const EXECUTOR_PANIC: &str = "the executor thread panicked";

/// The stage the run is on: the one running, else the head of the plan, else the template id.
fn current_stage(view: &NativeRunView) -> String {
    view.stages
        .iter()
        .find(|row| row.status == NativeStageStatus::Running)
        .map(|row| row.name.clone())
        .or_else(|| view.plan.first().and_then(|layer| layer.first()).cloned())
        .unwrap_or_else(|| view.template_id.clone())
}

/// The pull request a run works on, from its `pr_number`, `pr` or `target` input or its start target.
fn run_pr_number(view: &NativeRunView) -> Option<u64> {
    pr_workspace_holder::pr_number_of(&seeded_inputs(&view.request))
}

/// The vault a run keeps its directory in: none under `no_vault`, the request's own, else the global one.
fn run_vault(
    request: &StartNativeRun,
    workspace: &Path,
    default_vault: Option<&Path>,
) -> Option<Vault> {
    if request.no_vault {
        return None;
    }
    match request.vault.as_deref() {
        Some(root) => aidp_core::vault::resolve(Some(root), workspace),
        None => match default_vault {
            Some(root) if root.as_os_str().is_empty() => None,
            Some(root) => aidp_core::vault::resolve(Some(root), workspace),
            None => aidp_core::vault::resolve_global(),
        },
    }
}

/// Keeps the vault holder file of a run on a PR branch pointing at its current stage.
struct HolderWatch {
    finished: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl HolderWatch {
    /// Starts watching; inert when the run has no pull request or no vault is configured.
    fn start(store: &Arc<RunStore>, default_vault: Option<&Path>) -> HolderWatch {
        let finished = Arc::new(AtomicBool::new(false));
        let view = store.view();
        let target =
            run_pr_number(&view).zip(run_vault(&view.request, &view.workspace, default_vault));
        let Some((number, vault)) = target else {
            return HolderWatch {
                finished,
                handle: None,
            };
        };
        let segment = aidp_core::vault::repo_segment(&view.workspace);
        let watched = store.clone();
        let flag = finished.clone();
        let handle = std::thread::Builder::new()
            .name(format!("{THREAD_PREFIX}{HOLDER_THREAD_SUFFIX}"))
            .spawn(move || {
                let mut recorded: Option<String> = None;
                while !flag.load(Ordering::SeqCst) {
                    let view = watched.view();
                    let stage = current_stage(&view);
                    if recorded.as_deref() != Some(stage.as_str()) {
                        let holder = PrWorkspaceHolder {
                            stage: stage.clone(),
                            run_id: Some(view.run_id.clone()),
                            session_id: None,
                            acquired_at: crate::time::now_iso(),
                        };
                        match pr_workspace_holder::write(&vault.root, &segment, number, &holder) {
                            Ok(()) => recorded = Some(stage),
                            Err(error) => tracing::warn!(%error, "workspace holder not recorded"),
                        }
                    }
                    std::thread::sleep(DEFAULT_POLL);
                }
                pr_workspace_holder::remove(&vault.root, &segment, number);
            })
            .ok();
        HolderWatch { finished, handle }
    }

    /// Stops the watch and waits for the holder file to be removed.
    fn finish(self) {
        self.finished.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle {
            let _ = handle.join();
        }
    }
}

/// What native runs need from the daemon.
pub struct NativeDeps {
    pub default_vault: Option<PathBuf>,
    pub sessions: Arc<SessionManager>,
    pub projects: Arc<Projects>,
    pub bus: Arc<Bus>,
    /// Decides whether a stage session may start now; the guard waits, it never fails a stage.
    pub budget: Arc<dyn BudgetGuard>,
    /// Keeps a run on one executor thread.
    pub lock: Arc<dyn WorkspaceLock>,
    /// Run index file, see [`recovery::index_path`].
    pub index_file: PathBuf,
    /// Skill root holding `templates/` and `agents/`.
    pub skill_root: Option<PathBuf>,
    /// How often a stage session is polled for its end.
    pub poll_interval: Duration,
}

impl NativeDeps {
    pub fn new(
        sessions: Arc<SessionManager>,
        projects: Arc<Projects>,
        bus: Arc<Bus>,
        budget: Arc<dyn BudgetGuard>,
        lock: Arc<dyn WorkspaceLock>,
        state_dir: &Path,
        skill_root: Option<PathBuf>,
    ) -> Self {
        NativeDeps {
            default_vault: None,
            sessions,
            projects,
            bus,
            budget,
            lock,
            index_file: recovery::index_path(state_dir),
            skill_root,
            poll_interval: DEFAULT_POLL,
        }
    }

    /// Pins the host's vault; an empty path disables it instead of reading user configuration.
    pub fn with_default_vault(mut self, vault: PathBuf) -> Self {
        self.default_vault = Some(vault);
        self
    }
}

struct LiveRun {
    store: Arc<RunStore>,
    pipe: Arc<Pipeline>,
    control: Arc<RunControl>,
}

impl LiveRun {
    fn uses_external_agents(&self) -> bool {
        self.store.view().request.agent_executor == AgentExecutor::External
    }

    /// The agent board of the run, loaded from its directory on first use.
    fn open_board(&self) -> anyhow::Result<&agent_board::AgentBoard> {
        self.control.board.ensure_open(&self.store.view().run_dir)?;
        Ok(&self.control.board)
    }

    /// Closes every open agent job as cancelled; the run thread and late results see it.
    fn close_agent_jobs(&self) {
        if !self.uses_external_agents() {
            return;
        }
        let closed = self.open_board().and_then(|board| board.cancel_all());
        if let Err(error) = closed {
            tracing::warn!(%error, "agent jobs not closed");
        }
    }
}

/// Starts, tracks, controls and recovers native runs.
pub struct NativeRunManager {
    deps: Arc<NativeDeps>,
    live: Mutex<HashMap<String, Arc<LiveRun>>>,
    start_guard: Mutex<()>,
    executor_guard: Mutex<()>,
    stopping: AtomicBool,
    executors: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

fn not_found(message: String) -> anyhow::Error {
    SessionError::NotFound(message).into()
}

fn conflict(message: String) -> anyhow::Error {
    SessionError::Conflict(message).into()
}

/// The request inputs plus `target` seeded from the start target, and `jira_key`
/// too unless a non-empty `prompt` makes the run prompt-driven.
fn seeded_inputs(
    request: &StartNativeRun,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut inputs = request.inputs.clone();
    let prompt_driven = inputs
        .get(INPUT_PROMPT)
        .and_then(serde_json::Value::as_str)
        .is_some_and(|prompt| !prompt.trim().is_empty());
    if let Some(target) = request.target.as_ref().or(request.ticket.as_ref()) {
        let value = serde_json::Value::String(target.clone());
        inputs
            .entry(INPUT_TARGET.to_string())
            .or_insert_with(|| value.clone());
        if !prompt_driven {
            inputs.entry(INPUT_JIRA_KEY.to_string()).or_insert(value);
        }
    }
    inputs
}

/// The `--inputs` object of a run.
fn inputs_of(request: &StartNativeRun) -> serde_json::Value {
    serde_json::to_value(seeded_inputs(request)).unwrap_or_default()
}

impl NativeRunManager {
    pub fn new(deps: NativeDeps) -> Arc<Self> {
        Arc::new(NativeRunManager {
            deps: Arc::new(deps),
            live: Mutex::new(HashMap::new()),
            start_guard: Mutex::new(()),
            executor_guard: Mutex::new(()),
            stopping: AtomicBool::new(false),
            executors: Mutex::new(Vec::new()),
        })
    }

    /// Skill root holding the pipeline templates, when one is configured.
    pub fn skill_root(&self) -> Option<&Path> {
        self.deps.skill_root.as_deref()
    }

    pub(super) fn deps(&self) -> &Arc<NativeDeps> {
        &self.deps
    }

    /// Creates the run, with the same identity and vault folder `aidp run init` gives, and starts it.
    pub fn start(self: &Arc<Self>, request: StartNativeRun) -> anyhow::Result<String> {
        let _guard = lock(&self.start_guard);
        self.require_running()?;
        if let Some(client_request_id) = request.client_request_id.as_deref() {
            if client_request_id.trim().is_empty() {
                anyhow::bail!(crate::models::constants::flow_execution::EMPTY_CLIENT_REQUEST_ID);
            }
            if let Some(existing) = self
                .list()
                .into_iter()
                .find(|view| view.request.client_request_id.as_deref() == Some(client_request_id))
            {
                if serde_json::to_value(&existing.request)? == serde_json::to_value(&request)? {
                    return Ok(existing.run_id);
                }
                return Err(conflict(format!(
                    "client request `{client_request_id}` already has different parameters"
                )));
            }
        }
        crate::models::flow_execution_policy::validate_mcp_configuration(
            request.mcp_servers.as_deref(),
        )?;
        self.deps
            .sessions
            .mcp_configurations()
            .resolve(request.mcp_config_id.as_deref())?;
        let project = self
            .deps
            .projects
            .get(&request.project_id)
            .ok_or_else(|| not_found("project not found".to_string()))?;
        let workspace = request
            .workspace
            .clone()
            .unwrap_or_else(|| PathBuf::from(&project.path));
        let template = self.resolve_template(&request.template_id, &workspace)?;
        let vault = run_vault(&request, &workspace, self.deps.default_vault.as_deref());
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
        let mut state = run::init_with_mode(
            &template,
            &workspace,
            inputs_of(&request),
            vault.as_ref(),
            &ports,
            Some(run::NATIVE_RUN_MODE),
        )?;
        aidp_core::model_policy::apply(
            &mut state,
            vault.as_ref().map(|resolved| resolved.root.as_path()),
            &Default::default(),
        );
        run::save(&state)?;
        let run_id = state.run_id.clone();
        let run_dir = state.run_dir.clone();
        let store = Arc::new(RunStore::create(state, &request, workspace)?);
        recovery::index_add(&self.deps.index_file, &run_id, &run_dir)?;
        self.adopt(store)?;
        Ok(run_id)
    }

    fn resolve_template(&self, template: &str, workspace: &Path) -> anyhow::Result<PathBuf> {
        let direct = PathBuf::from(template);
        if direct.is_file() {
            return Ok(direct);
        }
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(root) = &self.deps.skill_root {
            roots.push(root.join(TEMPLATES_DIR));
        }
        roots.push(workspace.join(LOCAL_TEMPLATES_DIR));
        let refs: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
        pipeline::discover(&refs)?
            .into_iter()
            .find(|(id, _)| id == template)
            .map(|(_, path)| path)
            .ok_or_else(|| not_found(format!("pipeline not found: {template}")))
    }

    /// Registers a loaded run and starts its executor thread.
    pub(super) fn adopt(self: &Arc<Self>, store: Arc<RunStore>) -> anyhow::Result<()> {
        let live = self.register(store)?;
        self.spawn(live)
    }

    fn register(&self, store: Arc<RunStore>) -> anyhow::Result<Arc<LiveRun>> {
        let pipe = Arc::new(pipeline::load(&store.state().pipeline_path)?);
        let live = Arc::new(LiveRun {
            store,
            pipe,
            control: Arc::new(RunControl::default()),
        });
        lock(&self.live).insert(live.store.run_id(), live.clone());
        Ok(live)
    }

    /// The live run, or the run loaded from the index (registered, not started).
    fn live_or_load(&self, run_id: &str) -> anyhow::Result<Arc<LiveRun>> {
        if let Some(found) = lock(&self.live).get(run_id) {
            return Ok(found.clone());
        }
        let run_dir = recovery::index_read(&self.deps.index_file)
            .remove(run_id)
            .ok_or_else(|| not_found(format!("no run `{run_id}`")))?;
        self.register(Arc::new(RunStore::open(&run_dir)?))
    }

    fn spawn(self: &Arc<Self>, live: Arc<LiveRun>) -> anyhow::Result<()> {
        let _admission = lock(&self.executor_guard);
        self.require_running()?;
        let key = run_lock_key(&live.store.run_id());
        let started = Instant::now();
        while !self.deps.lock.try_acquire(&key) {
            if started.elapsed() > LOCK_WAIT {
                return Err(conflict(format!(
                    "run {} is still executing",
                    live.store.run_id()
                )));
            }
            std::thread::sleep(LOCK_RETRY);
        }
        let deps = self.deps.clone();
        let holder_watch = Arc::new(Mutex::new(Some(HolderWatch::start(
            &live.store,
            self.deps.default_vault.as_deref(),
        ))));
        let watch_for_thread = holder_watch.clone();
        let lock_key = key.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("{THREAD_PREFIX}{}", live.store.run_id()))
            .spawn(move || {
                let exec = RunExec::new(
                    deps.clone(),
                    live.store.clone(),
                    live.pipe.clone(),
                    live.control.clone(),
                );
                if threads::guarded(THREAD_NAME, || exec.run()).is_none() {
                    let _ = live
                        .store
                        .set_status(NativeRunStatus::Failed, Some(EXECUTOR_PANIC.to_string()));
                }
                if let Some(watch) = lock(&watch_for_thread).take() {
                    watch.finish();
                }
                deps.lock.release(&key);
            });
        match spawned {
            Ok(handle) => {
                let mut executors = lock(&self.executors);
                let mut index = 0;
                while index < executors.len() {
                    if executors[index].is_finished() {
                        let _ = executors.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
                executors.push(handle);
            }
            Err(error) => {
                if let Some(watch) = lock(&holder_watch).take() {
                    watch.finish();
                }
                self.deps.lock.release(&lock_key);
                return Err(error.into());
            }
        }
        Ok(())
    }

    fn require_running(&self) -> anyhow::Result<()> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err(SessionError::ShuttingDown.into());
        }
        Ok(())
    }

    /// Quiesces executors, preserves restart state, and releases every run lock before returning.
    pub fn shutdown(&self) {
        if let Err(error) = self.try_shutdown() {
            tracing::warn!(%error, "native manager shutdown could not confirm every agent stopped");
        }
    }

    /// Keeps suspended runs and owned executors available when agent shutdown needs a retry.
    pub fn try_shutdown(&self) -> anyhow::Result<()> {
        let _starts = lock(&self.start_guard);
        let _admission = lock(&self.executor_guard);
        self.stopping.store(true, Ordering::SeqCst);
        for live in lock(&self.live).values() {
            live.close_agent_jobs();
            live.store.freeze();
            live.control.suspend();
        }
        self.deps.sessions.try_shutdown()?;
        let mut join_failed = false;
        for handle in std::mem::take(&mut *lock(&self.executors)) {
            join_failed |= handle.join().is_err();
        }
        anyhow::ensure!(
            !join_failed,
            crate::models::constants::flow_shutdown::JOIN_FAILED
        );
        Ok(())
    }

    /// The run as the API shows it.
    pub fn get(&self, run_id: &str) -> Option<NativeRunView> {
        if let Some(found) = lock(&self.live).get(run_id) {
            return Some(found.store.view());
        }
        let run_dir = recovery::index_read(&self.deps.index_file).remove(run_id)?;
        RunStore::open(&run_dir).ok().map(|store| store.view())
    }

    /// Every known run, newest first.
    pub fn list(&self) -> Vec<NativeRunView> {
        let mut runs: Vec<NativeRunView> = recovery::index_read(&self.deps.index_file)
            .keys()
            .filter_map(|run_id| self.get(run_id))
            .collect();
        runs.sort_by(|left, right| right.started_at.cmp(&left.started_at));
        runs
    }

    /// Waits until the run is no longer executing: done, failed, cancelled, paused or at a gate.
    pub fn wait_for(&self, run_id: &str, timeout: Duration) -> Option<NativeRunView> {
        let started = Instant::now();
        loop {
            let view = self.get(run_id)?;
            let busy = self.deps.lock.is_busy(&run_lock_key(run_id));
            let settled = !matches!(
                view.status,
                NativeRunStatus::Pending | NativeRunStatus::Running
            );
            if (settled && !busy) || started.elapsed() > timeout {
                return Some(view);
            }
            std::thread::sleep(WAIT_POLL);
        }
    }

    /// Stops every live session of the run and marks it cancelled.
    pub fn cancel(self: &Arc<Self>, run_id: &str) -> anyhow::Result<()> {
        let _admission = lock(&self.start_guard);
        self.require_running()?;
        let live = self.live_or_load(run_id)?;
        if live.store.status().is_terminal() {
            return Err(conflict(format!("run {run_id} already ended")));
        }
        live.close_agent_jobs();
        live.control.cancel();
        for session_id in live.control.live_sessions() {
            if let Err(error) = self.deps.sessions.cancel(&session_id) {
                tracing::warn!(session = %session_id, %error, "session not cancelled");
            }
        }
        if !self.deps.lock.is_busy(&run_lock_key(run_id)) {
            let exec = RunExec::new(
                self.deps.clone(),
                live.store.clone(),
                live.pipe.clone(),
                live.control.clone(),
            );
            exec.cancel_idle()?;
        }
        Ok(())
    }

    /// Runs the failed or cancelled rows of `stage` again, and resumes the run.
    pub fn retry(self: &Arc<Self>, run_id: &str, stage: &str) -> anyhow::Result<()> {
        let _admission = lock(&self.start_guard);
        self.require_running()?;
        let live = self.live_or_load(run_id)?;
        self.require_paused(&live, run_id)?;
        let rows: Vec<String> = live
            .store
            .view()
            .stages
            .iter()
            .filter(|row| row.name == stage || row.name.starts_with(&format!("{stage}#")))
            .filter(|row| {
                matches!(
                    row.status,
                    NativeStageStatus::Failed | NativeStageStatus::Cancelled
                )
            })
            .map(|row| row.name.clone())
            .collect();
        if rows.is_empty() {
            return Err(not_found(format!("stage `{stage}` has nothing to retry")));
        }
        live.store.reset(&rows)?;
        live.store.set_status(NativeRunStatus::Running, None)?;
        self.spawn(live)
    }

    /// Runs every failed or cancelled row of a paused run again, and resumes it.
    pub fn retry_failed(self: &Arc<Self>, run_id: &str) -> anyhow::Result<()> {
        let _admission = lock(&self.start_guard);
        self.require_running()?;
        let live = self.live_or_load(run_id)?;
        self.require_paused(&live, run_id)?;
        let rows: Vec<String> = live
            .store
            .view()
            .stages
            .iter()
            .filter(|row| {
                matches!(
                    row.status,
                    NativeStageStatus::Failed | NativeStageStatus::Cancelled
                )
            })
            .map(|row| row.name.clone())
            .collect();
        if rows.is_empty() {
            return Err(not_found(format!("run `{run_id}` has nothing to retry")));
        }
        live.store.reset(&rows)?;
        live.store.set_status(NativeRunStatus::Running, None)?;
        self.spawn(live)
    }

    /// Skips `stage` and its unfinished rows, and resumes the run.
    pub fn skip(self: &Arc<Self>, run_id: &str, stage: &str) -> anyhow::Result<()> {
        let _admission = lock(&self.start_guard);
        self.require_running()?;
        let live = self.live_or_load(run_id)?;
        self.require_paused(&live, run_id)?;
        let rows: Vec<String> = live
            .store
            .view()
            .stages
            .iter()
            .filter(|row| row.name == stage || row.name.starts_with(&format!("{stage}#")))
            .filter(|row| !row.status.is_settled())
            .map(|row| row.name.clone())
            .collect();
        if rows.is_empty() {
            return Err(not_found(format!("stage `{stage}` has nothing to skip")));
        }
        for row in rows {
            live.store
                .skip(&row, NativeReason::SkippedByUser.as_str())?;
        }
        live.store.set_status(NativeRunStatus::Running, None)?;
        self.spawn(live)
    }

    fn require_paused(&self, live: &LiveRun, run_id: &str) -> anyhow::Result<()> {
        if live.store.status() == NativeRunStatus::Paused {
            return Ok(());
        }
        Err(conflict(format!("run {run_id} is not paused")))
    }

    /// Approves the gate the run waits at; the run goes on from there.
    pub fn approve(self: &Arc<Self>, run_id: &str, stage: &str) -> anyhow::Result<()> {
        let _admission = lock(&self.start_guard);
        self.require_running()?;
        let live = self.live_or_load(run_id)?;
        let view = live.store.view();
        if view.pending_gate.as_deref() != Some(stage) {
            return Err(conflict(format!("run {run_id} does not wait at `{stage}`")));
        }
        live.store.approve_gate(stage)?;
        if self.deps.lock.is_busy(&run_lock_key(run_id)) {
            live.control.wake();
            return Ok(());
        }
        live.store.set_status(NativeRunStatus::Running, None)?;
        self.spawn(live)
    }

    /// Resumes the runs that were executing when the daemon stopped. Call once at daemon start.
    pub fn resume_all(self: &Arc<Self>) -> usize {
        let _admission = lock(&self.start_guard);
        if self.require_running().is_err() {
            return 0;
        }
        recovery::resume_all(self)
    }

    /// The pending agent jobs of an external run, oldest first.
    pub fn agent_jobs(&self, run_id: &str) -> Vec<AgentJobSummary> {
        let Ok(live) = self.live_or_load(run_id) else {
            return Vec::new();
        };
        if !live.uses_external_agents() {
            return Vec::new();
        }
        live.open_board()
            .and_then(|board| board.summaries(crate::time::now_ms()))
            .unwrap_or_default()
    }

    /// Gives the job to a host for a lease.
    pub fn claim_agent_job(
        &self,
        run_id: &str,
        job_id: &str,
        request: &AgentClaimRequest,
    ) -> anyhow::Result<AgentClaim> {
        let live = self.external_run(run_id, job_id)?;
        let claim = live
            .open_board()?
            .claim(job_id, request, crate::time::now_ms())?;
        live.control.wake();
        Ok(claim)
    }

    /// Renews the lease of a claimed job.
    pub fn heartbeat_agent_job(
        &self,
        run_id: &str,
        job_id: &str,
        request: &AgentHeartbeatRequest,
    ) -> anyhow::Result<AgentHeartbeat> {
        let live = self.external_run(run_id, job_id)?;
        let beat =
            live.open_board()?
                .heartbeat(job_id, &request.claim_token, crate::time::now_ms())?;
        live.control.wake();
        Ok(beat)
    }

    /// Judges the result a host posted and wakes the run thread waiting for it.
    pub fn finish_agent_job(
        &self,
        run_id: &str,
        job_id: &str,
        result: &AgentJobResult,
    ) -> anyhow::Result<AgentJobAck> {
        let live = self.external_run(run_id, job_id)?;
        let ack = live
            .open_board()?
            .deliver(job_id, result, crate::time::now_ms())?;
        live.control.wake();
        Ok(ack)
    }

    /// True while a host holds a lease on an agent job of the run.
    pub fn has_claimed_agent_job(&self, run_id: &str) -> bool {
        self.live_or_load(run_id)
            .ok()
            .filter(|live| live.uses_external_agents())
            .and_then(|live| live.open_board().map(|board| board.has_claimed()).ok())
            .unwrap_or(false)
    }

    /// True when a host, not the gateway, launches the agent sessions of the run.
    pub fn uses_external_agents(&self, run_id: &str) -> bool {
        self.get(run_id)
            .is_some_and(|view| view.request.agent_executor == AgentExecutor::External)
    }

    /// The run named by `run_id` when it uses external agents and `job_id` belongs to it.
    fn external_run(&self, run_id: &str, job_id: &str) -> anyhow::Result<Arc<LiveRun>> {
        let live = self.live_or_load(run_id)?;
        if !live.uses_external_agents() {
            return Err(conflict(ERROR_NOT_EXTERNAL.to_string()));
        }
        match agent_board::parse_job_id(job_id) {
            Some((owner, _)) if owner == run_id => Ok(live),
            _ => Err(not_found(format!("{ERROR_FOREIGN_JOB} `{job_id}`"))),
        }
    }

    /// One inbox entry per run waiting at a gate.
    pub fn gate_inbox_entries(&self) -> Vec<InboxEntry> {
        self.list()
            .into_iter()
            .filter_map(|view| {
                let stage = view.pending_gate.clone()?;
                Some(InboxEntry {
                    kind: InboxKind::Gate.as_str().to_string(),
                    action: InboxAction::Open.as_str().to_string(),
                    title: format!("{}: approve {stage}", view.run_id),
                    detail: Some(view.template_id.clone()),
                    session_id: None,
                    fingerprint: Some(format!("{}:{stage}", view.run_id)),
                    automation_id: None,
                    project_id: Some(view.request.project_id.clone()),
                    project_name: None,
                    at: view.started_at.clone(),
                    urgency: 0,
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/native_runs_tests.rs"]
mod tests;
