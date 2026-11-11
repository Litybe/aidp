//! Action and probe registries, with the ports to sessions and budget.
//! Real actions are `aidp` subprocesses; owners fill the registries.

use crate::launcher::RunLauncher;
use crate::models::agent_claim::AgentClaim;
use crate::models::agent_claim_request::AgentClaimRequest;
use crate::models::agent_heartbeat::AgentHeartbeat;
use crate::models::agent_heartbeat_request::AgentHeartbeatRequest;
use crate::models::agent_job_ack::AgentJobAck;
use crate::models::agent_job_result::AgentJobResult;
use crate::models::agent_job_summary::AgentJobSummary;
use crate::models::constants::aidp::AIDP_PROGRAM;
use crate::models::constants::flow_facts::{OUTPUT_PR_BODY_FILE, OUTPUT_PR_FILE};
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::SessionStatus;
pub use crate::models::enums::{
    ActionErrorKind, BuiltinAction, BuiltinProbe, Engine, SessionStart, SessionState,
};
use crate::models::launched_run::LaunchedRun;
use crate::models::native_run_view::NativeRunView;
use crate::models::AidpCommand;
pub use crate::models::{
    ActionContext, CachedPrStatusSource, NoPrStatusSource, PipelineLaunch, PrStatusSource,
    ProbeContext, ProbeOutcome, ProjectRef, TicketLocator,
};
use crate::native_runs::dispatch::autonomous_env;
use crate::prs::PrStatusCache;
use crate::sessions::{Launch, Origin, OriginKind, SessionManager, StartRequest};
use aidp_core::process_runner::ProcessSpec;
use aidp_flows::{ActionOutcome, Catalog, PipelineRunStatus};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `with` key selecting the failure family of the `fail` action.
pub const WITH_KIND: &str = "kind";
/// `with` key carrying the message of the `fail` action.
pub const WITH_MESSAGE: &str = "message";
/// `with` key carrying the delay of the `timer` probe.
pub const WITH_SECONDS: &str = "seconds";
/// `with` key listing the gates a pipeline step pre-approves.
pub const WITH_AUTO_APPROVE_GATES: &str = "autoApproveGates";
/// `with` key switching a pipeline step to autonomous mode.
pub const WITH_AUTONOMOUS: &str = "autonomous";
/// `with` key choosing the engine of a pipeline step.
pub const WITH_ENGINE: &str = "engine";
/// Autonomous mode of a pipeline step whose `with` omits the key.
pub const DEFAULT_AUTONOMOUS: bool = true;

const DEFAULT_FAIL_MESSAGE: &str = "fail action";
const NO_STDERR: &str = "";
const ERROR_DETAIL_MAX_CHARS: usize = 2000;
const OUTPUT_RESULT: &str = "result";
const OUTPUT_STDOUT: &str = "stdout";
const OUTPUT_RUN_ID: &str = "runId";
const OUTPUT_SESSION_ID: &str = "sessionId";
const OUTPUT_RUN_DIR: &str = "runDir";
const NATIVE_OUTPUT_FILES: &[(&str, &str)] = &[
    (OUTPUT_PR_FILE, PR_FILE),
    (OUTPUT_PR_BODY_FILE, PR_BODY_FILE),
    (OUTPUT_FINDINGS, FINDINGS_FILE),
    (OUTPUT_VERIFY, VERIFY_FILE),
];
const OUTPUT_FINDINGS: &str = "findings";
const OUTPUT_VERIFY: &str = "verify";
const PR_FILE: &str = "pr.json";
const PR_BODY_FILE: &str = "pr-body.md";
const FINDINGS_FILE: &str = "findings.json";
const VERIFY_FILE: &str = "verify.json";
const RUN_STATUS_KEY: &str = "status";
const TICKET_PROJECT: &str = "project";
const TICKET_SERVICE: &str = "service";
const PROCESS_POLL: Duration = Duration::from_millis(20);
const DEFAULT_ACTION_TIMEOUT: Duration = Duration::from_secs(300);
const TIMEOUT_MESSAGE: &str = "timed out";
const SPAWN_FAILED_MESSAGE: &str = "cannot start the process";
const EXIT_CODES_BUSINESS: &[i32] = &[3, 4, 6];
const PLACEHOLDER_OPEN: char = '{';
const PLACEHOLDER_CLOSE: char = '}';
const PLACEHOLDER_KEY: &str = "key";
const MISSING_VALUE_MESSAGE: &str = "missing value for placeholder";

/// A deterministic verb of a flow. It reads the real state before it writes.
pub trait FlowAction: Send + Sync {
    /// True when the action modifies the ticket workspace: the executor
    /// then holds the workspace for the duration of the call.
    fn writes(&self) -> bool {
        false
    }

    /// Names of the `with` values the action reads, besides the ticket key: the
    /// executor fills the missing ones from the run's facts before it runs.
    fn placeholders(&self) -> Vec<String> {
        Vec::new()
    }

    fn run(&self, context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome;
}

/// A check for an external fact, polled until it holds.
pub trait WaitProbe: Send + Sync {
    fn poll(&self, context: &ProbeContext, with: &BTreeMap<String, Value>) -> ProbeOutcome;
}

/// Does nothing and returns its `with` as outputs.
pub struct NoopAction;

impl FlowAction for NoopAction {
    fn run(&self, _context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome {
        ActionOutcome::Ok(with.clone())
    }
}

/// Fails on purpose: a technical error, or a business one when `with.kind` is `business`.
pub struct FailAction;

impl FlowAction for FailAction {
    fn run(&self, _context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome {
        let message = with
            .get(WITH_MESSAGE)
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_FAIL_MESSAGE)
            .to_string();
        let business =
            with.get(WITH_KIND).and_then(Value::as_str) == Some(ActionErrorKind::Business.as_str());
        if business {
            ActionOutcome::BusinessError(message)
        } else {
            ActionOutcome::TechnicalError(message)
        }
    }
}

/// Satisfied once `with.seconds` have elapsed since the wait began.
pub struct TimerProbe;

impl WaitProbe for TimerProbe {
    fn poll(&self, context: &ProbeContext, with: &BTreeMap<String, Value>) -> ProbeOutcome {
        let seconds = with.get(WITH_SECONDS).and_then(Value::as_i64).unwrap_or(0);
        let elapsed = context.now_ms - context.started_ms >= seconds * 1000;
        if elapsed {
            ProbeOutcome::done(BTreeMap::new())
        } else {
            ProbeOutcome::pending(None)
        }
    }
}

/// An `aidp` subprocess with JSON output, run in the ticket workspace.
pub struct SubprocessAction {
    args: Vec<String>,
    timeout: Duration,
    business_codes: Vec<i32>,
    writes: bool,
}

impl SubprocessAction {
    pub fn new<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        SubprocessAction {
            args: args.into_iter().map(Into::into).collect(),
            timeout: DEFAULT_ACTION_TIMEOUT,
            business_codes: EXIT_CODES_BUSINESS.to_vec(),
            writes: false,
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Exit codes the action declares as business errors.
    pub fn business_codes(mut self, codes: &[i32]) -> Self {
        self.business_codes = codes.to_vec();
        self
    }

    /// Marks the action as a writer: the executor holds the workspace while it runs.
    pub fn holding_workspace(mut self) -> Self {
        self.writes = true;
        self
    }

    /// The arguments with `{key}` and every `{name}` filled from `with`.
    /// A placeholder with no value is an error naming it.
    pub fn resolve_args(
        &self,
        key: &str,
        with: &BTreeMap<String, Value>,
    ) -> Result<Vec<String>, String> {
        self.args
            .iter()
            .map(|argument| fill_placeholders(argument, key, with))
            .collect()
    }

    fn spec(&self, workspace: Option<&Path>, args: &[String]) -> ProcessSpec {
        let command = AidpCommand::json_action(args.iter().map(String::as_str));
        let mut spec = ProcessSpec::new(AIDP_PROGRAM).args(command.args().iter().cloned());
        spec.piped_stdout = true;
        spec.piped_stderr = true;
        spec.cwd = workspace.map(Path::to_path_buf);
        spec
    }
}

impl FlowAction for SubprocessAction {
    fn writes(&self) -> bool {
        self.writes
    }

    fn placeholders(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .args
            .iter()
            .flat_map(|argument| placeholder_names(argument))
            .filter(|name| name != PLACEHOLDER_KEY)
            .collect();
        names.dedup();
        names
    }

    fn run(&self, context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome {
        let args = match self.resolve_args(context.key, with) {
            Ok(args) => args,
            Err(message) => return ActionOutcome::TechnicalError(message),
        };
        let spec = self.spec(context.workspace, &args);
        let mut child = match context.runner.spawn(&spec) {
            Ok(child) => child,
            Err(error) => {
                return ActionOutcome::TechnicalError(format!("{SPAWN_FAILED_MESSAGE}: {error}"))
            }
        };
        let stdout_reader = child.take_stdout().map(drain);
        let stderr_reader = child.take_stderr().map(drain);
        let deadline = Instant::now() + self.timeout;
        let code = loop {
            match child.try_wait() {
                Ok(Some(code)) => break code,
                Ok(None) if Instant::now() >= deadline => {
                    if let Err(error) = child.kill().and_then(|()| child.wait().map(|_| ())) {
                        return ActionOutcome::TechnicalError(error.to_string());
                    }
                    if let Some(reader) = stdout_reader {
                        let _ = reader.join();
                    }
                    if let Some(reader) = stderr_reader {
                        let _ = reader.join();
                    }
                    return ActionOutcome::TechnicalError(format!(
                        "{TIMEOUT_MESSAGE} after {}s",
                        self.timeout.as_secs()
                    ));
                }
                Ok(None) => std::thread::sleep(PROCESS_POLL),
                Err(error) => return ActionOutcome::TechnicalError(error.to_string()),
            }
        };
        let stdout = stdout_reader
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();
        let stderr = stderr_reader
            .and_then(|handle| handle.join().ok())
            .unwrap_or_else(|| NO_STDERR.to_string());
        classify_exit(code, &stdout, &stderr, &self.business_codes)
    }
}

/// The `{name}` placeholders of one argument, in order.
fn placeholder_names(argument: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = argument;
    while let Some(open) = rest.find(PLACEHOLDER_OPEN) {
        let Some(close) = rest[open..].find(PLACEHOLDER_CLOSE) else {
            break;
        };
        names.push(rest[open + 1..open + close].to_string());
        rest = &rest[open + close + 1..];
    }
    names
}

/// Replaces `{key}` and `{name}` in one argument; an unknown name is an error.
fn fill_placeholders(
    argument: &str,
    key: &str,
    with: &BTreeMap<String, Value>,
) -> Result<String, String> {
    let mut filled = String::with_capacity(argument.len());
    let mut rest = argument;
    while let Some(open) = rest.find(PLACEHOLDER_OPEN) {
        let Some(close) = rest[open..].find(PLACEHOLDER_CLOSE) else {
            break;
        };
        let name = &rest[open + 1..open + close];
        filled.push_str(&rest[..open]);
        filled.push_str(&placeholder_value(name, key, with)?);
        rest = &rest[open + close + 1..];
    }
    filled.push_str(rest);
    Ok(filled)
}

fn placeholder_value(
    name: &str,
    key: &str,
    with: &BTreeMap<String, Value>,
) -> Result<String, String> {
    if name == PLACEHOLDER_KEY {
        return Ok(key.to_string());
    }
    match with.get(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Null) | None => Err(format!("{MISSING_VALUE_MESSAGE} `{name}`")),
        Some(other) => Ok(other.to_string()),
    }
}

/// Reads a child's output to its end on a helper thread.
fn drain(mut source: Box<dyn Read + Send>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = source.read_to_string(&mut text);
        text
    })
}

/// Maps an exit code to an outcome: 0 is success, a declared business code is escalated, other is failed.
pub fn classify_exit(
    code: i32,
    stdout: &str,
    stderr: &str,
    business_codes: &[i32],
) -> ActionOutcome {
    if code == 0 {
        return ActionOutcome::Ok(outputs_of(stdout));
    }
    let text = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    let text = if text.is_empty() {
        crate::models::constants::flow_execution::EMPTY_ACTION_ERROR
    } else {
        text
    };
    let text: String = text.chars().take(ERROR_DETAIL_MAX_CHARS).collect();
    let message = format!("exit {code}: {text}");
    if business_codes.contains(&code) {
        ActionOutcome::BusinessError(message)
    } else {
        ActionOutcome::TechnicalError(message)
    }
}

fn outputs_of(stdout: &str) -> BTreeMap<String, Value> {
    match serde_json::from_str::<Value>(stdout.trim()) {
        Ok(Value::Object(map)) => map.into_iter().collect(),
        Ok(other) => BTreeMap::from([(OUTPUT_RESULT.to_string(), other)]),
        Err(_) if stdout.trim().is_empty() => BTreeMap::new(),
        Err(_) => BTreeMap::from([(OUTPUT_STDOUT.to_string(), json!(stdout.trim()))]),
    }
}

/// Actions and wait probes by name.
#[derive(Clone, Default)]
pub struct Registry {
    actions: BTreeMap<String, Arc<dyn FlowAction>>,
    probes: BTreeMap<String, Arc<dyn WaitProbe>>,
    pr_status: Option<Arc<dyn PrStatusSource>>,
    wait_extensions: Arc<super::unity_wait::UnityWaitExtensions>,
}

impl Registry {
    /// The longer Unity compile waits granted to flow runs.
    pub fn wait_extensions(&self) -> Arc<super::unity_wait::UnityWaitExtensions> {
        self.wait_extensions.clone()
    }

    /// A registry holding `noop`, `fail` and the `timer` probe.
    pub fn with_builtins() -> Self {
        let mut registry = Registry::default();
        registry.register_action(BuiltinAction::Noop.as_str(), Arc::new(NoopAction));
        registry.register_action(BuiltinAction::Fail.as_str(), Arc::new(FailAction));
        registry.register_probe(BuiltinProbe::Timer.as_str(), Arc::new(TimerProbe));
        super::actions::register(&mut registry);
        super::probes::register(&mut registry);
        registry
    }

    /// A builtin registry whose probes read PR status through `cache` and `locate`.
    pub fn with_pr_status(cache: Arc<PrStatusCache>, locate: TicketLocator) -> Self {
        let mut registry = Registry::with_builtins();
        registry.set_pr_status_source(Arc::new(CachedPrStatusSource::new(cache, locate)));
        registry
    }

    /// Sets the PR status source handed to every probe poll.
    pub fn set_pr_status_source(&mut self, source: Arc<dyn PrStatusSource>) {
        self.pr_status = Some(source);
    }

    /// The PR status source, or one that knows no PR.
    pub fn pr_status_source(&self) -> Arc<dyn PrStatusSource> {
        self.pr_status
            .clone()
            .unwrap_or_else(|| Arc::new(NoPrStatusSource))
    }

    pub fn register_action(&mut self, name: &str, action: Arc<dyn FlowAction>) {
        self.actions.insert(name.to_string(), action);
    }

    pub fn register_probe(&mut self, name: &str, probe: Arc<dyn WaitProbe>) {
        self.probes.insert(name.to_string(), probe);
    }

    pub fn action(&self, name: &str) -> Option<Arc<dyn FlowAction>> {
        self.actions.get(name).cloned()
    }

    pub fn probe(&self, name: &str) -> Option<Arc<dyn WaitProbe>> {
        self.probes.get(name).cloned()
    }

    /// The names the flow validator checks `use` against.
    pub fn catalog(&self, pipelines: Option<BTreeSet<String>>) -> Catalog {
        Catalog {
            actions: self.actions.keys().cloned().collect(),
            wait_probes: self.probes.keys().cloned().collect(),
            pipelines,
        }
    }
}

/// The sessions as the executor drives them.
pub trait SessionPort: Send + Sync {
    /// Refuses unavailable runtime configuration before a flow request is saved.
    fn validate_mcp_reference(&self, configuration_id: Option<&str>) -> anyhow::Result<()> {
        anyhow::ensure!(
            configuration_id.is_none(),
            crate::models::constants::mcp_configuration::UNKNOWN_REFERENCE
        );
        Ok(())
    }

    /// The project a ticket belongs to, from its `project` or `service` field.
    fn project_for(&self, ticket: &Value) -> Option<ProjectRef>;

    /// The test checkout of the project of a ticket, when one is set: where `workspace: project`
    /// flows work instead of the project path.
    fn test_checkout_for(&self, _ticket: &Value) -> Option<PathBuf> {
        None
    }

    fn start(&self, launch: &PipelineLaunch) -> anyhow::Result<SessionStart>;

    /// Restarts the failed stages of a paused native run, so a retried step continues it.
    /// False when `session_id` is not a paused native run.
    fn resume_paused(&self, _session_id: &str) -> bool {
        false
    }

    fn state(&self, session_id: &str) -> SessionState;

    fn cancel(&self, session_id: &str);

    /// The pending agent jobs of a run whose sessions a host launches.
    fn agent_jobs(&self, _run_id: &str) -> Vec<AgentJobSummary> {
        Vec::new()
    }

    /// Gives an agent job of `run_id` to a host for a lease.
    fn claim_agent_job(
        &self,
        run_id: &str,
        _job_id: &str,
        _request: &AgentClaimRequest,
    ) -> anyhow::Result<AgentClaim> {
        Err(not_an_agent_run(run_id))
    }

    /// Renews the lease of a claimed agent job.
    fn heartbeat_agent_job(
        &self,
        run_id: &str,
        _job_id: &str,
        _request: &AgentHeartbeatRequest,
    ) -> anyhow::Result<AgentHeartbeat> {
        Err(not_an_agent_run(run_id))
    }

    /// Judges the result a host posted for an agent job.
    fn finish_agent_job(
        &self,
        run_id: &str,
        _job_id: &str,
        _result: &AgentJobResult,
    ) -> anyhow::Result<AgentJobAck> {
        Err(not_an_agent_run(run_id))
    }

    /// True while a host holds a lease on an agent job of the run.
    fn has_claimed_agent_job(&self, _run_id: &str) -> bool {
        false
    }
}

fn not_an_agent_run(run_id: &str) -> anyhow::Error {
    crate::session_error::SessionError::NotFound(format!("no run `{run_id}`")).into()
}

/// Sessions read to total the cost of a ticket.
const TICKET_SESSIONS_LIMIT: usize = 100_000;

/// Decides whether a new pipeline session may start now.
pub trait BudgetGuard: Send + Sync {
    fn allows(&self, template: &str) -> bool;
    /// The per-ticket ceiling `ticket` reached, `None` while it is within its ceilings.
    fn ticket_ceiling(&self, _ticket: &str) -> Option<crate::ticket_ceiling::CeilingHit> {
        None
    }
    /// Lifts the ceilings of `ticket` after a human approved to continue.
    fn raise_ticket_ceiling(&self, _ticket: &str) {}
}

/// A guard that never refuses.
pub struct AlwaysAllow;

impl BudgetGuard for AlwaysAllow {
    fn allows(&self, _template: &str) -> bool {
        true
    }
}

/// The dispatcher's budget verdict applied to flow launches: allowed unless
/// the rate-limit windows say wait or pause.
pub struct RateLimitGuard {
    sessions: Arc<SessionManager>,
    vault_root: PathBuf,
    clock: Arc<dyn aidp_core::clock::Clock>,
}

impl RateLimitGuard {
    pub fn new(
        sessions: Arc<SessionManager>,
        vault_root: PathBuf,
        clock: Arc<dyn aidp_core::clock::Clock>,
    ) -> Self {
        RateLimitGuard {
            sessions,
            vault_root,
            clock,
        }
    }
}

impl BudgetGuard for RateLimitGuard {
    fn ticket_ceiling(&self, ticket: &str) -> Option<crate::ticket_ceiling::CeilingHit> {
        let records = self.sessions.list(None, None, TICKET_SESSIONS_LIMIT);
        crate::ticket_ceiling::exceeded(&self.vault_root, ticket, &records)
    }

    fn raise_ticket_ceiling(&self, ticket: &str) {
        let records = self.sessions.list(None, None, TICKET_SESSIONS_LIMIT);
        crate::ticket_ceiling::raise(&self.vault_root, ticket, &records);
    }

    fn allows(&self, template: &str) -> bool {
        use aidp_tickets::budget::{self, Verdict};
        let snapshot = self.sessions.rate_limit().map(|info| {
            let window = |w: &crate::sessions::RateLimitWindow| budget::Window {
                utilization: w.utilization,
                resets_at_s: w.resets_at,
            };
            budget::RateSnapshot {
                status: info.status.clone(),
                is_using_overage: info.is_using_overage,
                five_hour: info.five_hour.as_ref().map(window),
                seven_day: info.seven_day.as_ref().map(window),
                observed_at_ms: info.observed_at,
            }
        });
        let config = aidp_tickets::config::load(&self.vault_root).budget;
        let stats = budget::BudgetStats::load(&self.vault_root);
        let mix = [template.to_string()];
        let verdict = budget::verdict(
            snapshot.as_ref(),
            0,
            0,
            &mix,
            &stats,
            &config,
            self.clock.now_ms(),
        );
        match verdict {
            Verdict::Allow { max_launches, .. } | Verdict::Conservative { max_launches, .. } => {
                max_launches > 0
            }
            Verdict::Wait { .. } | Verdict::Paused { .. } => false,
        }
    }
}

/// The real session port: launches v7 sessions through the [`SessionManager`].
pub struct ManagerSessions {
    manager: Arc<SessionManager>,
    projects: Arc<crate::projects::Projects>,
    launcher: Arc<RunLauncher>,
    vault_root: Option<PathBuf>,
}

impl ManagerSessions {
    pub fn new(
        manager: Arc<SessionManager>,
        projects: Arc<crate::projects::Projects>,
        launcher: Arc<RunLauncher>,
    ) -> Self {
        ManagerSessions {
            manager,
            projects,
            launcher,
            vault_root: None,
        }
    }

    pub fn with_vault_root(mut self, vault_root: PathBuf) -> Self {
        self.vault_root = Some(vault_root);
        self
    }

    fn run_outcome(&self, session_id: &str) -> Option<(String, BTreeMap<String, Value>)> {
        let record = self.manager.get(session_id)?;
        let run = crate::runs::resolve_run(&record.cwd, record.workflow_run_id.as_deref());
        let Some(run) = run else {
            let status = match record.status {
                SessionStatus::Done => PipelineRunStatus::Done,
                SessionStatus::Cancelled => PipelineRunStatus::Cancelled,
                _ => PipelineRunStatus::Failed,
            };
            return Some((
                status.as_str().to_string(),
                session_outputs(session_id, None, None),
            ));
        };
        let status = crate::runs::normalize(&run.run_dir)
            .and_then(|view| view[RUN_STATUS_KEY].as_str().map(str::to_string))
            .unwrap_or_else(|| PipelineRunStatus::Failed.as_str().to_string());
        let outputs = session_outputs(session_id, Some(&run.run_id), Some(&run.run_dir));
        Some((status, outputs))
    }
}

/// A native run as the executor sees it: running until it settles, then ended.
pub(crate) fn native_session_state(view: &NativeRunView) -> SessionState {
    match view.status {
        NativeRunStatus::Pending | NativeRunStatus::Running | NativeRunStatus::WaitingGate => {
            SessionState::Running
        }
        NativeRunStatus::Done => native_ended(view, PipelineRunStatus::Done),
        NativeRunStatus::Cancelled => native_ended(view, PipelineRunStatus::Cancelled),
        NativeRunStatus::Failed | NativeRunStatus::Paused => {
            native_ended(view, PipelineRunStatus::Failed)
        }
    }
}

fn native_ended(view: &NativeRunView, status: PipelineRunStatus) -> SessionState {
    SessionState::Ended {
        run_status: status.as_str().to_string(),
        outputs: native_outputs(view),
    }
}

/// The run id and dir, the path of each `pr.json`, `pr-body.md`, `findings.json` and `verify.json` a stage wrote,
/// then the keys of the last stage's `result.json` (see [`super::step_outputs`]).
fn native_outputs(view: &NativeRunView) -> BTreeMap<String, Value> {
    let mut outputs = session_outputs_for_run(&view.run_id, &view.run_dir);
    let mut stage_dirs: Vec<PathBuf> = std::fs::read_dir(&view.run_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    stage_dirs.sort();
    for (key, file) in NATIVE_OUTPUT_FILES {
        let found = stage_dirs
            .iter()
            .map(|dir| dir.join(file))
            .find(|path| path.is_file());
        if let Some(path) = found {
            outputs.insert((*key).to_string(), json!(path.to_string_lossy()));
        }
    }
    super::step_outputs::merge_stage_outputs(&mut outputs, view);
    outputs
}

fn session_outputs_for_run(run_id: &str, run_dir: &Path) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (OUTPUT_RUN_ID.to_string(), json!(run_id)),
        (OUTPUT_RUN_DIR.to_string(), json!(run_dir.to_string_lossy())),
    ])
}

fn session_outputs(
    session_id: &str,
    run_id: Option<&str>,
    run_dir: Option<&Path>,
) -> BTreeMap<String, Value> {
    let mut outputs = BTreeMap::new();
    outputs.insert(OUTPUT_SESSION_ID.to_string(), json!(session_id));
    if let Some(run_id) = run_id {
        outputs.insert(OUTPUT_RUN_ID.to_string(), json!(run_id));
    }
    if let Some(run_dir) = run_dir {
        outputs.insert(OUTPUT_RUN_DIR.to_string(), json!(run_dir.to_string_lossy()));
    }
    outputs
}

impl SessionPort for ManagerSessions {
    fn validate_mcp_reference(&self, configuration_id: Option<&str>) -> anyhow::Result<()> {
        self.manager
            .mcp_configurations()
            .resolve(configuration_id)?;
        Ok(())
    }

    fn test_checkout_for(&self, ticket: &Value) -> Option<PathBuf> {
        let name = [TICKET_PROJECT, TICKET_SERVICE]
            .iter()
            .find_map(|field| ticket.get(*field).and_then(Value::as_str))?;
        self.projects.get(name)?.test_checkout.map(PathBuf::from)
    }

    fn project_for(&self, ticket: &Value) -> Option<ProjectRef> {
        let name = [TICKET_PROJECT, TICKET_SERVICE]
            .iter()
            .find_map(|field| ticket.get(*field).and_then(Value::as_str))?;
        let project = self.projects.get(name)?;
        let path = PathBuf::from(&project.path);
        let repo = aidp_core::vault::repo_segment(&path);
        Some(ProjectRef {
            id: project.id,
            path,
            repo,
        })
    }

    fn start(&self, launch: &PipelineLaunch) -> anyhow::Result<SessionStart> {
        let vault = self
            .vault_root
            .clone()
            .or_else(|| aidp_core::vault::resolve_global().map(|vault| vault.root));
        if let Some(vault) = vault {
            let workspace = super::lock::workspace_key(&vault, &launch.project.repo, &launch.key);
            if self.manager.is_workspace_busy(&workspace) {
                return Ok(SessionStart::Busy);
            }
        }
        match self
            .launcher
            .launch(pipeline_start_request(launch), launch.autonomous)?
        {
            LaunchedRun::Native { run_id } => Ok(SessionStart::Started(run_id)),
            LaunchedRun::Session(Launch::Started(record)) => Ok(SessionStart::Started(record.id)),
            LaunchedRun::Session(Launch::Queued(queued)) => {
                self.manager.queue_cancel(&queued.id);
                Ok(SessionStart::Busy)
            }
        }
    }

    fn resume_paused(&self, session_id: &str) -> bool {
        self.launcher.native().retry_failed(session_id).is_ok()
    }

    fn state(&self, session_id: &str) -> SessionState {
        if let Some(view) = self.launcher.native().get(session_id) {
            return native_session_state(&view);
        }
        let Some(record) = self.manager.get(session_id) else {
            return SessionState::Gone;
        };
        match record.status {
            SessionStatus::Running | SessionStatus::Queued => SessionState::Running,
            SessionStatus::Orphaned => SessionState::Gone,
            _ => match self.run_outcome(session_id) {
                Some((run_status, outputs)) => SessionState::Ended {
                    run_status,
                    outputs,
                },
                None => SessionState::Gone,
            },
        }
    }

    fn cancel(&self, session_id: &str) {
        if self.launcher.native().get(session_id).is_some() {
            let _ = self.launcher.native().cancel(session_id);
            return;
        }
        let _ = self.manager.cancel(session_id);
    }

    fn agent_jobs(&self, run_id: &str) -> Vec<AgentJobSummary> {
        self.launcher.native().agent_jobs(run_id)
    }

    fn claim_agent_job(
        &self,
        run_id: &str,
        job_id: &str,
        request: &AgentClaimRequest,
    ) -> anyhow::Result<AgentClaim> {
        self.launcher
            .native()
            .claim_agent_job(run_id, job_id, request)
    }

    fn heartbeat_agent_job(
        &self,
        run_id: &str,
        job_id: &str,
        request: &AgentHeartbeatRequest,
    ) -> anyhow::Result<AgentHeartbeat> {
        self.launcher
            .native()
            .heartbeat_agent_job(run_id, job_id, request)
    }

    fn finish_agent_job(
        &self,
        run_id: &str,
        job_id: &str,
        result: &AgentJobResult,
    ) -> anyhow::Result<AgentJobAck> {
        self.launcher
            .native()
            .finish_agent_job(run_id, job_id, result)
    }

    fn has_claimed_agent_job(&self, run_id: &str) -> bool {
        self.launcher.native().has_claimed_agent_job(run_id)
    }
}

/// The session request of a pipeline step; autonomous steps export the autonomous switch.
pub fn pipeline_start_request(launch: &PipelineLaunch) -> StartRequest {
    let env = if launch.autonomous {
        autonomous_env()
    } else {
        BTreeMap::new()
    };
    StartRequest {
        project_id: launch.project.id.clone(),
        template_id: Some(launch.template.clone()),
        target: Some(launch.ticket_key.clone()),
        provider: launch.provider.clone(),
        permission_mode: launch.permission_mode.clone(),
        strict_mcp_config: launch.strict_mcp_config,
        mcp_config_id: launch.mcp_config_id.clone(),
        mcp_servers: launch.mcp_servers.clone(),
        denied_shell_commands: launch.denied_shell_commands.clone(),
        denied_mcp_tools: launch.denied_mcp_tools.clone(),
        headless: launch.headless,
        origin: Some(Origin {
            kind: OriginKind::Flow.as_str().to_string(),
            r#ref: Some(launch.ticket_key.clone()),
            label: Some(launch.step.clone()),
        }),
        auto_approve_gates: launch.auto_approve_gates.clone(),
        inputs_file: Some(launch.inputs_file.to_string_lossy().to_string()),
        worktree: (!launch.project_checkout).then(|| launch.key.clone()),
        engine: launch.engine.map(|engine| engine.as_str().to_string()),
        agent_executor: launch.agent_executor,
        env,
        ..StartRequest::default()
    }
}

/// `with` as a JSON object, for inputs files.
pub fn with_as_object(with: &BTreeMap<String, Value>) -> Map<String, Value> {
    with.iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/flow_registry_wiring_tests.rs"]
mod wiring_tests;
