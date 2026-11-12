//! Executor driven in-process: temp vault, fake sessions, scripted processes.
//! A restart is a second engine over the same directories and sessions.

use crate::bus::Bus;
use crate::flows::executor::{ascii_json, ticket_status_of, Ports};
use crate::flows::lock::{workspace_key, LocalLock, WorkspaceLock};
use crate::flows::recovery::{resume_all, tick};
use crate::flows::registry::{
    classify_exit, ActionContext, BudgetGuard, BuiltinAction, FlowAction, ManagerSessions,
    PipelineLaunch, ProbeContext, ProbeOutcome, ProjectRef, Registry, SessionPort, SessionStart,
    SessionState, SubprocessAction, WaitProbe,
};
use crate::flows::{FlowEngine, FlowEngineConfig, FlowEngineError};
use crate::models::enums::engine::Engine;
use crate::models::enums::SessionStatus;
use crate::models::pr_workspace_holder::PrWorkspaceHolder;
use crate::pr_workspace_holder;
use crate::projects::Projects;
use crate::sessions::{Launch, SessionManager, StartRequest};
use crate::worktrees::{Workspace, WorkspaceKind};
use aidp_core::clock::Clock;
use aidp_core::id::IdGen;
use aidp_core::process_runner::{ProcessRunner, ScriptedOutput, ScriptedProcessRunner};
use aidp_flows::{
    ActionOutcome, FlowDef, FlowState, FlowStatus, ManualRequest, Step, StepKind, StepStatus,
};
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEPLOY_APPROVAL_GATE: &str = "deploy_approval";
const MANUAL_LABEL: &str = "manual-0";
const MANUAL_BUSY_LINE: &str = "manual-0 not started: workspace busy";
const REPO: &str = "shared-repo";
const SERVICE: &str = "svc";
const ACTOR: &str = "alice";
const STEP_PREPARE: &str = "prepare";
const STEP_DEVELOP: &str = "develop";
const STEP_COOLDOWN: &str = "cooldown";
const STEP_APPROVE: &str = "approve";
const RUN_DONE: &str = "done";
const WAIT_LIMIT: Duration = Duration::from_secs(10);
const WAIT_POLL: Duration = Duration::from_millis(10);
const SHORT_LOCK_WAIT: Duration = Duration::from_millis(60);
const START_EPOCH_MS: i64 = 1_790_000_000_000;
const MINUTE_MS: i64 = 61_000;
const WRITE_ACTION: &str = "write";
const COUNT_ACTION: &str = "count";
const SCRIPTED_PROBE: &str = "scripted";
const STEP_WATCH: &str = "watch";
const OUTPUT_CONCLUSION: &str = "conclusion";
const OUTPUT_SEEN: &str = "seen";
const STATE_POLLS: &str = "polls";
const CONCLUSION_SUCCESS: &str = "success";
const AUTONOMOUS_KEY: &str = "autonomous";
const AUTONOMOUS_OPT_OUT: bool = false;
const PIPELINE_TEMPLATE: &str = "dev-aidp";
const HOLDER_ACTION: &str = "holder-probe";
const INPUT_PR_TEXT: &str = "pr";
const INPUT_PR_NUMBER_VALUE: &str = "pr_number";
const PR_AS_TEXT: &str = "#41";
const PR_AS_TEXT_NUMBER: u64 = 41;
const PR_AS_NUMBER: u64 = 42;
const STEP_HOLDER: &str = "holder-step";
const EXECUTION_KEY: &str = "TEST-91-run-a";
const STEP_MERGE_ID: &str = "merge";
const MERGE_TICKET: &str = "LOC-77";
const MERGE_OUTPUT_MERGED: &str = "merged";
const MERGE_OUTPUT_CONFLICT: &str = "conflict";
const SIBLING_EXECUTION_KEY: &str = "TEST-91-run-b";
const BUSINESS_TICKET_KEY: &str = "TEST-91";
const EXECUTION_FLOW: &str = "test-flow";
const DENIED_SHELL_COMMAND: &str = "rm";
const DENIED_MCP_SERVER: &str = "documents";
const DENIED_MCP_TOOL: &str = "delete";
const UNSUPPORTED_MCP_DEFINITION: &str = "unsupported-definition";
const MANAGER_TICKET_KEY: &str = "TEST-21";
const EXECUTION_FLOW_YAML: &str = include_str!("../fixtures/flows/test-flow.yaml");

#[test]
fn it_execution_request_recovers_a_state_persisted_before_start_event() {
    let harness = Harness::new(EXECUTION_KEY);
    let engine = harness.engine();
    let request = execution_request(false, true);
    let mut state = FlowState::new(
        EXECUTION_KEY,
        FlowDef::from_yaml(EXECUTION_FLOW_YAML).unwrap(),
    );
    state.settings.extend(
        request
            .settings
            .as_ref()
            .unwrap()
            .as_object()
            .unwrap()
            .clone(),
    );
    let runtime = crate::models::Runtime {
        start_request: Some(request.clone()),
        ..Default::default()
    };
    engine
        .executor()
        .store
        .save_runtime(REPO, EXECUTION_KEY, &runtime)
        .unwrap();
    engine
        .executor()
        .store
        .save_state(REPO, &state, &engine.executor().now())
        .unwrap();
    let restarted = harness.engine();
    restarted.start_request(request).unwrap();
    wait_step(
        &restarted,
        EXECUTION_KEY,
        STEP_APPROVE,
        StepStatus::WaitingHuman,
    );
    assert_eq!(harness.runs.load(Ordering::SeqCst), 1);
    restarted.abort(EXECUTION_KEY, ACTOR).unwrap();
}

fn execution_request(pipeline: bool, human: bool) -> crate::models::TicketFlowRequest {
    crate::models::TicketFlowRequest {
        execution_key: Some(EXECUTION_KEY.to_string()),
        ticket: ticket(BUSINESS_TICKET_KEY),
        flow: Some(EXECUTION_FLOW.to_string()),
        settings: settings(pipeline, false, human),
        policy: Some(crate::models::flow_execution_policy::FlowExecutionPolicy {
            provider: Some(aidp_agents::config::CLAUDE_PROVIDER.to_string()),
            permission_mode: Some(
                crate::models::enums::PermissionMode::DontAsk
                    .as_str()
                    .to_string(),
            ),
            strict_mcp_config: Some(true),
            mcp_servers: Some(Vec::new()),
            denied_shell_commands: vec![DENIED_SHELL_COMMAND.to_string()],
            denied_mcp_tools: BTreeMap::from([(
                DENIED_MCP_SERVER.to_string(),
                vec![DENIED_MCP_TOOL.to_string()],
            )]),
            headless: true,
            ..Default::default()
        }),
        entry: None,
    }
}

#[test]
fn it_execution_request_is_idempotent_concurrently_after_completion_and_restart() {
    let harness = Harness::new(EXECUTION_KEY);
    let engine = harness.engine();
    let request = execution_request(false, true);
    std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..6)
            .map(|_| scope.spawn(|| engine.start_request(request.clone()).unwrap()))
            .collect();
        for task in tasks {
            assert_eq!(task.join().unwrap().key, EXECUTION_KEY);
        }
    });
    wait_step(
        &engine,
        EXECUTION_KEY,
        STEP_APPROVE,
        StepStatus::WaitingHuman,
    );
    assert_eq!(harness.runs.load(Ordering::SeqCst), 1);
    engine.approve(EXECUTION_KEY, STEP_APPROVE, ACTOR).unwrap();
    wait_for(RUN_DONE, || {
        run_status(&engine, EXECUTION_KEY) == FlowStatus::Done
    });
    wait_for(RUN_DONE, || engine.executor().is_idle());
    let restarted = harness.engine();
    let state = restarted.start_request(request.clone()).unwrap();
    assert_eq!(state.status, FlowStatus::Done);
    assert_eq!(state.ticket, ticket(BUSINESS_TICKET_KEY));
    assert_eq!(harness.runs.load(Ordering::SeqCst), 2);
    let mut changed = request;
    changed.policy.as_mut().unwrap().autonomous = true;
    assert!(conflict_of(&restarted.start_request(changed).unwrap_err()));
}

#[test]
fn a_retried_start_with_another_executor_conflicts() {
    let harness = Harness::new("executor_mode_conflict");
    let engine = harness.engine();
    let request = execution_request(false, true);
    engine.start_request(request.clone()).unwrap();
    let mut external = request;
    external.policy.as_mut().unwrap().executor = crate::models::enums::FlowExecutorMode::External;

    let error = engine.start_request(external).unwrap_err();

    assert!(conflict_of(&error), "{error}");
}

#[test]
fn it_sibling_executions_keep_ticket_identity_and_override_pipeline_policy() {
    let harness = Harness::new(SIBLING_EXECUTION_KEY);
    let engine = harness.engine();
    let request = execution_request(true, false);
    engine.start_request(request.clone()).unwrap();
    let mut sibling = request;
    sibling.execution_key = Some(SIBLING_EXECUTION_KEY.to_string());
    engine.start_request(sibling).unwrap();
    wait_for(STEP_DEVELOP, || harness.sessions.launch_count() == 2);
    let launches = harness.sessions.launches.lock().unwrap().clone();
    for launch in launches {
        assert_eq!(launch.ticket_key, BUSINESS_TICKET_KEY);
        assert!(!launch.autonomous);
        assert_eq!(launch.auto_approve_gates, Some(Vec::new()));
        assert_eq!(
            launch.provider.as_deref(),
            Some(aidp_agents::config::CLAUDE_PROVIDER)
        );
        assert!(launch.headless);
        let start = crate::flows::registry::pipeline_start_request(&launch);
        assert!(start.headless);
        assert_eq!(start.target.as_deref(), Some(BUSINESS_TICKET_KEY));
        assert_eq!(start.worktree.as_deref(), Some(launch.key.as_str()));
        assert_eq!(start.strict_mcp_config, Some(true));
        assert_eq!(start.mcp_servers, Some(Vec::new()));
        assert_eq!(
            start.denied_shell_commands,
            vec![DENIED_SHELL_COMMAND.to_string()]
        );
        assert_eq!(
            start.denied_mcp_tools[DENIED_MCP_SERVER],
            vec![DENIED_MCP_TOOL.to_string()]
        );
        assert_eq!(
            start.permission_mode.as_deref(),
            Some(crate::models::enums::PermissionMode::DontAsk.as_str())
        );
    }
    assert_ne!(
        harness.sessions.workspace_keys()[0],
        harness.sessions.workspace_keys()[1]
    );
    engine.abort(EXECUTION_KEY, ACTOR).unwrap();
    engine.abort(SIBLING_EXECUTION_KEY, ACTOR).unwrap();
}

#[test]
fn it_unsupported_mcp_definitions_are_refused_before_flow_persistence() {
    let harness = Harness::new(UNSUPPORTED_MCP_DEFINITION);
    let engine = harness.engine();
    let mut request = execution_request(false, false);
    request.policy.as_mut().unwrap().mcp_servers = Some(vec![json!(UNSUPPORTED_MCP_DEFINITION)]);
    let error = engine.start_request(request).unwrap_err();
    assert_eq!(
        error.to_string(),
        crate::models::constants::flow_execution::UNSUPPORTED_MCP_DEFINITIONS
    );
    assert!(engine.list(None, None).is_empty());
}

/// Counts how many times it ran, optionally holding the workspace while it does.
struct CountingAction {
    runs: Arc<AtomicUsize>,
    writes: bool,
}

impl FlowAction for CountingAction {
    fn writes(&self) -> bool {
        self.writes
    }

    fn run(&self, _context: &ActionContext<'_>, _with: &BTreeMap<String, Value>) -> ActionOutcome {
        self.runs.fetch_add(1, Ordering::SeqCst);
        ActionOutcome::Ok(BTreeMap::new())
    }
}

/// Records the holder file as it exists while the action runs, for one pull request.
struct HolderProbeAction {
    vault: PathBuf,
    number: u64,
    seen: Arc<Mutex<Option<PrWorkspaceHolder>>>,
}

impl FlowAction for HolderProbeAction {
    fn writes(&self) -> bool {
        true
    }

    fn run(&self, _context: &ActionContext<'_>, _with: &BTreeMap<String, Value>) -> ActionOutcome {
        *self.seen.lock().unwrap() = pr_workspace_holder::read(&self.vault, REPO, self.number);
        ActionOutcome::Ok(BTreeMap::new())
    }
}

/// Satisfied at once with a conclusion, or on the second poll echoing the state it kept.
struct ScriptedProbe {
    immediate: bool,
}

impl WaitProbe for ScriptedProbe {
    fn poll(&self, context: &ProbeContext, _with: &BTreeMap<String, Value>) -> ProbeOutcome {
        if self.immediate {
            let outputs =
                BTreeMap::from([(OUTPUT_CONCLUSION.to_string(), json!(CONCLUSION_SUCCESS))]);
            return ProbeOutcome::done(outputs);
        }
        match &context.last_state {
            None => ProbeOutcome::pending(Some(json!({ STATE_POLLS: 1 }))),
            Some(state) => {
                let outputs = BTreeMap::from([(OUTPUT_SEEN.to_string(), state.clone())]);
                ProbeOutcome::done(outputs)
            }
        }
    }
}

/// A clock the test moves by hand.
struct ManualClock(AtomicI64);

impl ManualClock {
    fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_utc(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(self.now_ms()).expect("valid timestamp")
    }

    fn now_rfc3339(&self) -> String {
        self.now_utc().to_rfc3339()
    }

    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Unique ids, so two launches never share an inputs file.
struct CountingIds(AtomicUsize);

impl IdGen for CountingIds {
    fn new_run_id(&self, _clock: &dyn Clock) -> String {
        format!("id{}", self.0.fetch_add(1, Ordering::SeqCst))
    }
}

struct Switch(Arc<AtomicBool>, Arc<Ceiling>);

/// The per-ticket ceiling a test turns on; `raised` counts the approvals that lifted it.
#[derive(Default)]
struct Ceiling {
    hit: AtomicBool,
    raised: AtomicUsize,
}

impl BudgetGuard for Switch {
    fn allows(&self, _template: &str) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn ticket_ceiling(&self, _ticket: &str) -> Option<crate::ticket_ceiling::CeilingHit> {
        self.1
            .hit
            .load(Ordering::SeqCst)
            .then(|| crate::ticket_ceiling::CeilingHit {
                what: "tokens",
                used: 12,
                limit: 10,
                prompt: "Plafond atteint".to_string(),
            })
    }

    fn raise_ticket_ceiling(&self, _ticket: &str) {
        self.1.raised.fetch_add(1, Ordering::SeqCst);
        self.1.hit.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, PartialEq)]
enum FakeSession {
    Running,
    Ended(String),
    Gone,
}

/// Sessions that live in memory and survive an engine "restart".
struct FakeSessions {
    vault: PathBuf,
    lock: Arc<LocalLock>,
    sessions: Mutex<BTreeMap<String, (FakeSession, String)>>,
    launches: Mutex<Vec<PipelineLaunch>>,
    paused: Mutex<Vec<String>>,
}

impl FakeSessions {
    fn finish(&self, session_id: &str, run_status: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        let entry = sessions.get_mut(session_id).expect("known session");
        entry.0 = FakeSession::Ended(run_status.to_string());
        self.lock.release(&entry.1);
    }

    /// Ends a session as a failed run that the host could still resume.
    fn pause(&self, session_id: &str) {
        self.finish(session_id, "failed");
        self.paused.lock().unwrap().push(session_id.to_string());
    }

    fn forget(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        let entry = sessions.get_mut(session_id).expect("known session");
        entry.0 = FakeSession::Gone;
        self.lock.release(&entry.1);
    }

    fn launch_count(&self) -> usize {
        self.launches.lock().unwrap().len()
    }

    fn session_of(&self, index: usize) -> String {
        format!("s{index}")
    }

    fn workspace_keys(&self) -> Vec<String> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .map(|(_, key)| key.clone())
            .collect()
    }

    fn is_running(&self, session_id: &str) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .get(session_id)
            .is_some_and(|(state, _)| *state == FakeSession::Running)
    }
}

impl SessionPort for FakeSessions {
    fn project_for(&self, _ticket: &Value) -> Option<ProjectRef> {
        Some(ProjectRef {
            id: "p1".to_string(),
            path: self.vault.clone(),
            repo: REPO.to_string(),
        })
    }

    fn start(&self, launch: &PipelineLaunch) -> anyhow::Result<SessionStart> {
        let key = workspace_key(&self.vault, &launch.project.repo, &launch.key);
        if !self.lock.try_acquire(&key) {
            return Ok(SessionStart::Busy);
        }
        let mut sessions = self.sessions.lock().unwrap();
        let id = format!("s{}", sessions.len());
        sessions.insert(id.clone(), (FakeSession::Running, key));
        self.launches.lock().unwrap().push(launch.clone());
        Ok(SessionStart::Started(id))
    }

    fn state(&self, session_id: &str) -> SessionState {
        let sessions = self.sessions.lock().unwrap();
        match sessions.get(session_id).map(|(state, _)| state.clone()) {
            Some(FakeSession::Running) => SessionState::Running,
            Some(FakeSession::Ended(run_status)) => SessionState::Ended {
                run_status,
                outputs: BTreeMap::new(),
            },
            Some(FakeSession::Gone) | None => SessionState::Gone,
        }
    }

    fn resume_paused(&self, session_id: &str) -> bool {
        let mut paused = self.paused.lock().unwrap();
        let Some(position) = paused.iter().position(|id| id == session_id) else {
            return false;
        };
        paused.remove(position);
        let mut sessions = self.sessions.lock().unwrap();
        let entry = sessions.get_mut(session_id).expect("known session");
        assert!(self.lock.try_acquire(&entry.1));
        entry.0 = FakeSession::Running;
        true
    }

    fn cancel(&self, session_id: &str) {
        self.finish(session_id, "cancelled");
    }
}

struct Harness {
    workspace: TempWorkspace,
    sessions: Arc<FakeSessions>,
    clock: Arc<ManualClock>,
    budget: Arc<AtomicBool>,
    ceiling: Arc<Ceiling>,
    bus: Arc<Bus>,
    runs: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
    restore: Mutex<Option<Arc<dyn FlowAction>>>,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let workspace = TempWorkspace::new(tag);
        let vault = workspace.child("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let lock = Arc::new(LocalLock::default());
        Harness {
            sessions: Arc::new(FakeSessions {
                vault,
                lock,
                sessions: Mutex::new(BTreeMap::new()),
                launches: Mutex::new(Vec::new()),
                paused: Mutex::new(Vec::new()),
            }),
            workspace,
            clock: Arc::new(ManualClock(AtomicI64::new(START_EPOCH_MS))),
            budget: Arc::new(AtomicBool::new(true)),
            ceiling: Arc::new(Ceiling::default()),
            bus: Arc::new(Bus::new()),
            runs: Arc::new(AtomicUsize::new(0)),
            writes: Arc::new(AtomicUsize::new(0)),
            restore: Mutex::new(None),
        }
    }

    fn engine(&self) -> Arc<FlowEngine> {
        self.engine_with_probe(None)
    }

    fn engine_with_probe(&self, probe: Option<ScriptedProbe>) -> Arc<FlowEngine> {
        let mut registry = Registry::with_builtins();
        if let Some(probe) = probe {
            registry.register_probe(SCRIPTED_PROBE, Arc::new(probe));
        }
        registry.register_action(
            COUNT_ACTION,
            Arc::new(CountingAction {
                runs: self.runs.clone(),
                writes: false,
            }),
        );
        registry.register_action(
            WRITE_ACTION,
            Arc::new(CountingAction {
                runs: self.writes.clone(),
                writes: true,
            }),
        );
        if let Some(restore) = self.restore.lock().unwrap().clone() {
            registry.register_action(
                crate::models::constants::flow_actions::ACTION_GIT_RESTORE,
                restore,
            );
        }
        let ports = Ports {
            sessions: self.sessions.clone(),
            budget: Arc::new(Switch(self.budget.clone(), self.ceiling.clone())),
            lock: self.sessions.lock.clone(),
            bus: self.bus.clone(),
            clock: self.clock.clone(),
            ids: Arc::new(CountingIds(AtomicUsize::new(0))),
            runner: Arc::new(ScriptedProcessRunner::new()),
            lock_wait: SHORT_LOCK_WAIT,
        };
        let config = FlowEngineConfig {
            flow_dirs: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flows")],
            ..FlowEngineConfig::default()
        };
        FlowEngine::new(
            self.sessions.vault.clone(),
            self.workspace.child("state"),
            registry,
            ports,
            config,
        )
    }
}

fn ticket(key: &str) -> Value {
    json!({ "key": key, "service": SERVICE, "source": "jira" })
}

fn wait_for(description: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT_LIMIT;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(WAIT_POLL);
    }
}

fn step_status(engine: &FlowEngine, key: &str, step: &str) -> Option<StepStatus> {
    engine.get(key).ok()?.steps.get(step).map(|s| s.status)
}

fn run_status(engine: &FlowEngine, key: &str) -> FlowStatus {
    engine.get(key).unwrap().status
}

/// Waits until the executor recorded the effect, so a "restart" never overlaps its work.
fn wait_effect(engine: &FlowEngine, key: &str, effect: &str) {
    wait_for(&format!("effect {effect}"), || {
        engine
            .get(key)
            .is_ok_and(|state| state.effects_applied.contains(effect))
    });
    wait_for("the engine to go idle", || engine.executor().is_idle());
}

fn wait_step(engine: &FlowEngine, key: &str, step: &str, status: StepStatus) {
    wait_for(&format!("{step} to be {status:?}"), || {
        step_status(engine, key, step) == Some(status)
    });
}

fn conflict_of(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<FlowEngineError>(),
        Some(FlowEngineError::Conflict { .. })
    )
}

fn settings(pipeline: bool, wait: bool, human: bool) -> Option<Value> {
    Some(json!({ "with_pipeline": pipeline, "with_wait": wait, "with_human": human }))
}

fn single_step_flow(step: Step) -> FlowDef {
    FlowDef {
        id: "inline".to_string(),
        version: 1,
        steps: vec![step],
        ..FlowDef::default()
    }
}

fn wait_step_def(id: &str, probe: &str) -> Step {
    Step {
        use_: Some(probe.to_string()),
        ..Step::new(id, StepKind::Wait)
    }
}

fn action_step(id: &str, name: &str) -> Step {
    Step {
        use_: Some(name.to_string()),
        ..Step::new(id, StepKind::Action)
    }
}

#[test]
fn it_full_run_goes_through_pipeline_human_and_noop_steps() {
    let harness = Harness::new("flows_full_run");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-1"), Some("test-flow"), None)
        .unwrap();

    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    wait_for("the session", || harness.sessions.launch_count() == 1);
    harness
        .sessions
        .finish(&harness.sessions.session_of(0), RUN_DONE);
    tick(&engine);

    wait_step(&engine, "LOC-1", STEP_APPROVE, StepStatus::WaitingHuman);
    assert_eq!(run_status(&engine, "LOC-1"), FlowStatus::WaitingHuman);
    let inbox = engine.inbox_entries();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "flow.human");

    engine.approve("LOC-1", STEP_APPROVE, ACTOR).unwrap();
    wait_for("the run to end", || {
        run_status(&engine, "LOC-1") == FlowStatus::Done
    });
    assert_eq!(harness.runs.load(Ordering::SeqCst), 2);
    assert!(engine.inbox_entries().is_empty());

    let journal = engine.journal("LOC-1").unwrap();
    assert!(journal
        .iter()
        .any(|l| l.line.contains(ACTOR) && l.line.contains("approve")));
    wait_for("the lifecycle events", || {
        let events: Vec<String> = harness
            .bus
            .recent(200, 0)
            .iter()
            .filter_map(|e| e["type"].as_str().map(str::to_string))
            .collect();
        [
            "flow.started",
            "flow.step_changed",
            "flow.waiting_you",
            "flow.ended",
        ]
        .iter()
        .all(|expected| events.iter().any(|event| event == expected))
    });
}

#[test]
fn it_pipeline_inputs_are_written_as_ascii_escaped_json() {
    let harness = Harness::new("flows_inputs");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-2"), Some("test-flow"), None)
        .unwrap();
    wait_for("the session", || harness.sessions.launch_count() == 1);

    let launch = harness.sessions.launches.lock().unwrap()[0].clone();
    let text = std::fs::read_to_string(&launch.inputs_file).unwrap();
    assert!(text.is_ascii());
    assert!(text.contains("D\\u00e9veloppement"));
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["label"], "Développement ébauche");
    assert_eq!(parsed["feature"], "LOC-2");
    assert_eq!(launch.step, STEP_DEVELOP);
}

fn first_launch_autonomous(tag: &str, key: &str, with: BTreeMap<String, Value>) -> bool {
    let harness = Harness::new(tag);
    let engine = harness.engine();
    let step = Step {
        use_: Some(PIPELINE_TEMPLATE.to_string()),
        with,
        ..Step::new(STEP_DEVELOP, StepKind::Pipeline)
    };
    engine
        .start_with_def(ticket(key), single_step_flow(step), None)
        .unwrap();
    wait_for("the session", || harness.sessions.launch_count() == 1);
    let autonomous = harness.sessions.launches.lock().unwrap()[0].autonomous;
    autonomous
}

#[test]
fn pipeline_step_is_autonomous_when_the_key_is_absent() {
    assert!(first_launch_autonomous(
        "flows_auto_default",
        "LOC-30",
        BTreeMap::new()
    ));
}

#[test]
fn pipeline_step_explicit_false_stays_not_autonomous() {
    let with = BTreeMap::from([(AUTONOMOUS_KEY.to_string(), Value::Bool(AUTONOMOUS_OPT_OUT))]);
    assert!(!first_launch_autonomous("flows_auto_false", "LOC-31", with));
}

#[test]
fn ascii_json_escapes_astral_characters_as_surrogate_pairs() {
    let text = ascii_json(&json!({ "emoji": "\u{1F600}" }));
    assert!(text.is_ascii());
    assert!(text.contains("\\ud83d\\ude00"));
}

#[test]
fn it_second_start_for_the_same_ticket_conflicts_with_the_existing_run() {
    let harness = Harness::new("flows_single_run");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-3"), Some("test-flow"), None)
        .unwrap();

    let error = engine
        .start(ticket("LOC-3"), Some("test-flow"), None)
        .unwrap_err();
    match error.downcast_ref::<FlowEngineError>() {
        Some(FlowEngineError::Conflict { existing, .. }) => {
            let existing = existing.as_ref().expect("the existing run");
            assert_eq!(existing["key"], "LOC-3");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
}

#[test]
fn it_decision_on_a_step_that_is_not_waiting_conflicts() {
    let harness = Harness::new("flows_wrong_state");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-4"), Some("test-flow"), None)
        .unwrap();
    wait_step(&engine, "LOC-4", STEP_DEVELOP, StepStatus::Running);

    let error = engine.approve("LOC-4", STEP_APPROVE, ACTOR).unwrap_err();
    assert!(conflict_of(&error));
    let error = engine.approve("LOC-4", STEP_DEVELOP, ACTOR).unwrap_err();
    assert!(conflict_of(&error));
    let missing = engine.approve("LOC-404", STEP_APPROVE, ACTOR).unwrap_err();
    assert!(matches!(
        missing.downcast_ref::<FlowEngineError>(),
        Some(FlowEngineError::NotFound(_))
    ));
}

#[test]
fn it_pause_lets_the_running_step_finish_then_stops() {
    let harness = Harness::new("flows_pause");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-5"), Some("test-flow"), None)
        .unwrap();
    wait_step(&engine, "LOC-5", STEP_DEVELOP, StepStatus::Running);

    engine.pause("LOC-5", ACTOR).unwrap();
    assert_eq!(run_status(&engine, "LOC-5"), FlowStatus::Pausing);
    harness
        .sessions
        .finish(&harness.sessions.session_of(0), RUN_DONE);
    tick(&engine);

    wait_for("the run to pause", || {
        run_status(&engine, "LOC-5") == FlowStatus::Paused
    });
    assert_eq!(
        step_status(&engine, "LOC-5", STEP_DEVELOP),
        Some(StepStatus::Done)
    );
    assert_eq!(step_status(&engine, "LOC-5", STEP_APPROVE), None);

    engine.resume("LOC-5", ACTOR).unwrap();
    wait_step(&engine, "LOC-5", STEP_APPROVE, StepStatus::WaitingHuman);
}

#[test]
fn it_abort_cancels_the_running_session() {
    let harness = Harness::new("flows_abort");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-6"), Some("test-flow"), None)
        .unwrap();
    wait_for("the session", || harness.sessions.launch_count() == 1);
    let session = harness.sessions.session_of(0);

    engine.abort("LOC-6", ACTOR).unwrap();
    assert_eq!(run_status(&engine, "LOC-6"), FlowStatus::Aborted);
    assert!(!harness.sessions.is_running(&session));
    assert!(engine
        .start(ticket("LOC-6"), Some("test-flow"), None)
        .is_ok());
}

#[test]
fn it_restart_during_a_human_step_keeps_the_inbox_entry() {
    let harness = Harness::new("flows_restart_human");
    let first = harness.engine();
    first
        .start(
            ticket("LOC-7"),
            Some("test-flow"),
            settings(false, false, true),
        )
        .unwrap();
    wait_step(&first, "LOC-7", STEP_APPROVE, StepStatus::WaitingHuman);
    wait_effect(&first, "LOC-7", "approve:0:create_inbox");
    drop(first);

    let second = harness.engine();
    assert_eq!(resume_all(&second), 1);
    assert_eq!(second.inbox_entries().len(), 1);
    assert_eq!(run_status(&second, "LOC-7"), FlowStatus::WaitingHuman);

    second.approve("LOC-7", STEP_APPROVE, ACTOR).unwrap();
    wait_for("the run to end", || {
        run_status(&second, "LOC-7") == FlowStatus::Done
    });
    assert_eq!(harness.runs.load(Ordering::SeqCst), 2);
}

#[test]
fn it_restart_with_a_session_finished_meanwhile_reads_its_outcome() {
    let harness = Harness::new("flows_restart_finished");
    let first = harness.engine();
    first
        .start(ticket("LOC-8"), Some("test-flow"), None)
        .unwrap();
    wait_effect(&first, "LOC-8", "develop:0:start_pipeline");
    drop(first);

    harness
        .sessions
        .finish(&harness.sessions.session_of(0), RUN_DONE);
    let second = harness.engine();
    resume_all(&second);

    wait_step(&second, "LOC-8", STEP_APPROVE, StepStatus::WaitingHuman);
    assert_eq!(
        step_status(&second, "LOC-8", STEP_DEVELOP),
        Some(StepStatus::Done)
    );
    assert_eq!(harness.sessions.launch_count(), 1);
}

#[test]
fn it_restart_with_a_vanished_session_fails_the_step_as_orphaned_and_retry_relaunches() {
    let harness = Harness::new("flows_restart_vanished");
    let first = harness.engine();
    first
        .start(ticket("LOC-9"), Some("test-flow"), None)
        .unwrap();
    wait_effect(&first, "LOC-9", "develop:0:start_pipeline");
    drop(first);

    harness.sessions.forget(&harness.sessions.session_of(0));
    let second = harness.engine();
    resume_all(&second);

    let state: FlowState = second.get("LOC-9").unwrap();
    assert_eq!(state.status, FlowStatus::Failed);
    let develop = &state.steps[STEP_DEVELOP];
    assert_eq!(develop.status, StepStatus::Failed);
    assert_eq!(develop.reason.as_deref(), Some("orphaned"));
    assert_eq!(
        ticket_status_of(&state),
        Some(aidp_tickets::model::Status::Blocked)
    );

    second.retry("LOC-9", STEP_DEVELOP, ACTOR).unwrap();
    wait_for("a second launch", || harness.sessions.launch_count() == 2);
    assert_eq!(
        step_status(&second, "LOC-9", STEP_DEVELOP),
        Some(StepStatus::Running)
    );
}

#[test]
fn it_retry_of_a_failed_pipeline_step_continues_its_paused_run() {
    let harness = Harness::new("flows_retry_paused_run");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-21"), Some("test-flow"), None)
        .unwrap();
    wait_effect(&engine, "LOC-21", "develop:0:start_pipeline");
    let run = harness.sessions.session_of(0);
    harness.sessions.pause(&run);
    resume_all(&engine);
    wait_step(&engine, "LOC-21", STEP_DEVELOP, StepStatus::Failed);

    engine.retry("LOC-21", STEP_DEVELOP, ACTOR).unwrap();
    wait_effect(&engine, "LOC-21", "develop:1:start_pipeline");
    assert_eq!(harness.sessions.launch_count(), 1);
    assert!(harness.sessions.is_running(&run));

    harness.sessions.finish(&run, RUN_DONE);
    resume_all(&engine);
    wait_step(&engine, "LOC-21", STEP_APPROVE, StepStatus::WaitingHuman);
    assert_eq!(
        step_status(&engine, "LOC-21", STEP_DEVELOP),
        Some(StepStatus::Done)
    );
    assert_eq!(harness.sessions.launch_count(), 1);
}

#[test]
fn it_restart_during_a_wait_resumes_it_without_replaying_done_steps() {
    let harness = Harness::new("flows_restart_wait");
    let first = harness.engine();
    first
        .start(
            ticket("LOC-10"),
            Some("test-flow"),
            settings(false, true, false),
        )
        .unwrap();
    wait_step(&first, "LOC-10", STEP_COOLDOWN, StepStatus::WaitingExternal);
    wait_effect(&first, "LOC-10", "cooldown:0:start_wait");
    assert_eq!(harness.runs.load(Ordering::SeqCst), 1);
    drop(first);

    let second = harness.engine();
    resume_all(&second);
    assert_eq!(
        step_status(&second, "LOC-10", STEP_COOLDOWN),
        Some(StepStatus::WaitingExternal)
    );
    assert_eq!(harness.runs.load(Ordering::SeqCst), 1);

    harness.clock.advance(MINUTE_MS);
    tick(&second);
    wait_for("the run to end", || {
        run_status(&second, "LOC-10") == FlowStatus::Done
    });
    assert_eq!(harness.runs.load(Ordering::SeqCst), 2);
    assert_eq!(
        step_status(&second, "LOC-10", STEP_PREPARE),
        Some(StepStatus::Done)
    );
}

#[test]
fn it_insufficient_budget_parks_the_pipeline_step_until_it_returns() {
    let harness = Harness::new("flows_budget");
    harness.budget.store(false, Ordering::SeqCst);
    let engine = harness.engine();
    engine
        .start(ticket("LOC-11"), Some("test-flow"), None)
        .unwrap();

    wait_step(&engine, "LOC-11", STEP_DEVELOP, StepStatus::WaitingExternal);
    let state = engine.get("LOC-11").unwrap();
    assert_eq!(state.steps[STEP_DEVELOP].reason.as_deref(), Some("budget"));
    assert_eq!(state.status, FlowStatus::WaitingExternal);
    assert_eq!(harness.sessions.launch_count(), 0);

    harness.budget.store(true, Ordering::SeqCst);
    tick(&engine);
    wait_for("the launch", || harness.sessions.launch_count() == 1);
    assert_eq!(
        step_status(&engine, "LOC-11", STEP_DEVELOP),
        Some(StepStatus::Running)
    );
}

#[test]
fn it_two_tickets_of_one_repo_run_their_pipelines_in_parallel() {
    let harness = Harness::new("flows_parallel");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-12"), Some("test-flow"), None)
        .unwrap();
    engine
        .start(ticket("LOC-13"), Some("test-flow"), None)
        .unwrap();

    wait_for("both sessions", || harness.sessions.launch_count() == 2);
    assert!(harness.sessions.is_running("s0") && harness.sessions.is_running("s1"));
    let keys = harness.sessions.workspace_keys();
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
}

#[test]
fn it_a_writing_action_waits_for_the_workspace_then_runs() {
    let harness = Harness::new("flows_lock");
    let engine = harness.engine();
    let held = workspace_key(&harness.sessions.vault, REPO, "LOC-14");
    assert!(harness.sessions.lock.try_acquire(&held));

    let flow = single_step_flow(action_step("write-step", WRITE_ACTION));
    engine.start_with_def(ticket("LOC-14"), flow, None).unwrap();
    std::thread::sleep(SHORT_LOCK_WAIT * 4);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        step_status(&engine, "LOC-14", "write-step"),
        Some(StepStatus::Running)
    );

    harness.sessions.lock.release(&held);
    tick(&engine);
    wait_for("the run to end", || {
        run_status(&engine, "LOC-14") == FlowStatus::Done
    });
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
    assert!(!harness.sessions.lock.is_busy(&held));
}

#[test]
fn it_failing_actions_map_to_failed_escalated_or_a_warning() {
    let harness = Harness::new("flows_fail");
    let engine = harness.engine();
    let fail = |kind: &str| Step {
        use_: Some(BuiltinAction::Fail.as_str().to_string()),
        with: BTreeMap::from([("kind".to_string(), json!(kind))]),
        ..Step::new("boom", StepKind::Action)
    };

    engine
        .start_with_def(ticket("LOC-15"), single_step_flow(fail("technical")), None)
        .unwrap();
    wait_for("failure", || {
        run_status(&engine, "LOC-15") == FlowStatus::Failed
    });

    engine
        .start_with_def(ticket("LOC-16"), single_step_flow(fail("business")), None)
        .unwrap();
    wait_for("escalation", || {
        run_status(&engine, "LOC-16") == FlowStatus::Escalated
    });

    let mut tolerated = fail("technical");
    tolerated.on_error = aidp_flows::OnError::Warn;
    let flow = FlowDef {
        steps: vec![tolerated, action_step("after", COUNT_ACTION)],
        ..single_step_flow(action_step("unused", COUNT_ACTION))
    };
    engine.start_with_def(ticket("LOC-17"), flow, None).unwrap();
    wait_for("the run to end", || {
        run_status(&engine, "LOC-17") == FlowStatus::Done
    });
    assert_eq!(
        engine.get("LOC-17").unwrap().steps["boom"].warnings.len(),
        1
    );
}

#[test]
fn it_manual_action_is_journaled_and_runs_beside_the_flow() {
    let harness = Harness::new("flows_manual");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-18"), Some("test-flow"), None)
        .unwrap();
    wait_step(&engine, "LOC-18", STEP_DEVELOP, StepStatus::Running);

    let request = ManualRequest::Action {
        name: COUNT_ACTION.to_string(),
        with: BTreeMap::new(),
    };
    engine.manual("LOC-18", request, ACTOR).unwrap();
    wait_for("the manual action", || {
        engine
            .journal("LOC-18")
            .unwrap()
            .iter()
            .any(|line| line.line.contains("manual effect"))
    });
    assert_eq!(
        step_status(&engine, "LOC-18", STEP_DEVELOP),
        Some(StepStatus::Running)
    );
}

#[test]
fn it_state_is_written_before_effects_and_list_filters_by_status() {
    let harness = Harness::new("flows_store");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-19"), Some("test-flow"), None)
        .unwrap();
    wait_step(&engine, "LOC-19", STEP_DEVELOP, StepStatus::Running);

    let ticket_dir = harness
        .sessions
        .vault
        .join("Pipelines")
        .join(REPO)
        .join("LOC-19");
    wait_for("state and journal files", || {
        ticket_dir.join("flow/state.json").is_file()
            && ticket_dir.join("flow/journal.jsonl").is_file()
    });
    let note = std::fs::read_to_string(ticket_dir.join("_flow.md")).unwrap();
    assert!(note.starts_with("---\n") && note.contains("| develop |"));

    assert_eq!(engine.list(Some("running"), Some(SERVICE)).len(), 1);
    assert!(engine.list(Some("done"), None).is_empty());
    assert!(engine.list(None, Some("other")).is_empty());

    wait_for("the pipeline effect to be recorded", || {
        engine.get("LOC-19").is_ok_and(|state| {
            state
                .effects_applied
                .iter()
                .any(|id| id.ends_with("start_pipeline"))
        })
    });
}

#[test]
fn subprocess_exit_codes_map_to_outcomes() {
    let business = [4];
    assert_eq!(
        classify_exit(5, "", "", &business),
        ActionOutcome::TechnicalError(format!(
            "exit 5: {}",
            crate::models::constants::flow_execution::EMPTY_ACTION_ERROR
        ))
    );
    assert!(matches!(
        classify_exit(0, "{\"ok\":true}", "", &business),
        ActionOutcome::Ok(outputs) if outputs["ok"] == json!(true)
    ));
    assert!(matches!(
        classify_exit(4, "", "refused", &business),
        ActionOutcome::BusinessError(_)
    ));
    assert!(matches!(
        classify_exit(5, "", "boom", &business),
        ActionOutcome::TechnicalError(_)
    ));
    assert!(matches!(
        classify_exit(0, "plain", "", &business),
        ActionOutcome::Ok(outputs) if outputs["stdout"] == json!("plain")
    ));
}

#[test]
fn subprocess_action_runs_through_the_process_runner() {
    let runner: Arc<dyn ProcessRunner> = Arc::new(
        ScriptedProcessRunner::new()
            .with_output(ScriptedOutput {
                status_code: 0,
                ..ScriptedOutput::default()
            })
            .with_spawn_stdout(b"{\"pr\":42}".to_vec()),
    );
    let action = SubprocessAction::new(["gh", "pr-view"]).holding_workspace();
    let context = ActionContext {
        key: "LOC-20",
        workspace: None,
        runner: runner.as_ref(),
    };
    assert!(action.writes());
    match action.run(&context, &BTreeMap::new()) {
        ActionOutcome::Ok(outputs) => assert_eq!(outputs["pr"], json!(42)),
        other => panic!("unexpected outcome {other:?}"),
    }
}

#[test]
fn session_admission_waits_for_a_held_workspace() {
    let workspace = TempWorkspace::new("flows_admission");
    let state_dir = workspace.child("state");
    let project_dir = workspace.child("project");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::create_dir_all(&project_dir).unwrap();
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(
        ScriptedProcessRunner::new().with_spawn_stdout(b"{\"type\":\"result\"}\n".to_vec()),
    );
    let manager =
        SessionManager::load_with_runner(&state_dir, Arc::new(Bus::new()), projects, runner);
    let checkout = Workspace {
        kind: WorkspaceKind::Project,
        path: project_dir.clone(),
        branch: None,
        name: None,
    }
    .key();

    assert!(manager.try_hold_workspace(&checkout));
    assert!(!manager.try_hold_workspace(&checkout));
    assert!(manager.is_workspace_busy(&checkout));
    let request = StartRequest {
        project_id: project.id,
        engine: Some("prompt".to_string()),
        prompt: Some("hello".to_string()),
        ..StartRequest::default()
    };
    assert!(matches!(manager.start(request).unwrap(), Launch::Queued(_)));

    manager.release_workspace(&checkout);
    wait_for("the queued launch to finish", || {
        let done = manager
            .list(None, None, 10)
            .iter()
            .any(|record| record.status == SessionStatus::Done);
        done && !manager.is_workspace_busy(&checkout)
    });
}

#[test]
fn manager_sessions_start_a_flow_origin_session_in_the_ticket_workspace() {
    let _vault_off = EnvGuard::set("AIDP_VAULT", "off");
    let workspace = TempWorkspace::new("flows_manager_sessions");
    let state_dir = workspace.child("state");
    let project_dir = workspace.child("project");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::create_dir_all(&project_dir).unwrap();
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(
        ScriptedProcessRunner::new().with_spawn_stdout(b"{\"type\":\"result\"}\n".to_vec()),
    );
    let manager = SessionManager::load_with_runner(
        &state_dir,
        Arc::new(Bus::new()),
        projects.clone(),
        runner,
    );
    let native = crate::native_runs::NativeRunManager::new(crate::native_runs::NativeDeps::new(
        manager.clone(),
        projects.clone(),
        Arc::new(Bus::new()),
        Arc::new(crate::flows::registry::AlwaysAllow),
        Arc::new(crate::models::LocalLock::default()),
        &state_dir,
        None,
    ));
    let launcher = Arc::new(crate::launcher::RunLauncher::new(manager.clone(), native));
    let port = ManagerSessions::new(manager.clone(), projects, launcher);

    let reference = port
        .project_for(&json!({ "key": MANAGER_TICKET_KEY, "service": project.name }))
        .expect("the project of the ticket");
    assert_eq!(reference.id, project.id);
    let inputs_file = workspace.child("inputs.json");
    let launch = PipelineLaunch {
        key: MANAGER_TICKET_KEY.to_string(),
        ticket_key: MANAGER_TICKET_KEY.to_string(),
        provider: None,
        permission_mode: None,
        strict_mcp_config: None,
        mcp_config_id: None,
        mcp_servers: None,
        denied_shell_commands: Vec::new(),
        denied_mcp_tools: BTreeMap::new(),
        step: STEP_DEVELOP.to_string(),
        template: "dev-aidp".to_string(),
        project: reference,
        inputs_file: inputs_file.clone(),
        auto_approve_gates: None,
        autonomous: false,
        headless: false,
        engine: Some(Engine::Workflow),
        agent_executor: crate::models::enums::agent_executor::AgentExecutor::Internal,
        project_checkout: false,
    };
    let SessionStart::Started(session_id) = port.start(&launch).unwrap() else {
        panic!("expected a started session");
    };

    let record = manager.get(&session_id).expect("the session record");
    assert_eq!(record.origin.kind, "flow");
    assert_eq!(record.origin.r#ref.as_deref(), Some(MANAGER_TICKET_KEY));
    assert_eq!(record.origin.label.as_deref(), Some(STEP_DEVELOP));
    assert!(record.prompt.contains(inputs_file.to_str().unwrap()));
    wait_for("the session to end", || {
        matches!(port.state(&session_id), SessionState::Ended { .. })
    });
    match port.state(&session_id) {
        SessionState::Ended { run_status, .. } => assert_eq!(run_status, RUN_DONE),
        other => panic!("unexpected state {other:?}"),
    }
    assert_eq!(port.state("unknown"), SessionState::Gone);
}

/// Starts a `workspace: project` pipeline step of a project that has (or has not) a test checkout
/// and returns the cwd of the session, with the project dir and the clone dir.
fn project_flow_session_cwd(tag: &str, with_test_checkout: bool) -> (String, PathBuf, PathBuf) {
    let _vault_off = EnvGuard::set("AIDP_VAULT", "off");
    let workspace = TempWorkspace::new(tag);
    let state_dir = workspace.child("state");
    let project_dir = workspace.child("project");
    let clone_dir = workspace.child("test-clone");
    for dir in [&state_dir, &project_dir, &clone_dir] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    if with_test_checkout {
        projects
            .set_test_checkout(&project.id, clone_dir.to_str())
            .unwrap();
    }
    let runner: Arc<dyn ProcessRunner> = Arc::new(
        ScriptedProcessRunner::new().with_spawn_stdout(
            b"{\"type\":\"result\"}
"
            .to_vec(),
        ),
    );
    let manager = SessionManager::load_with_runner(
        &state_dir,
        Arc::new(Bus::new()),
        projects.clone(),
        runner,
    );
    let native = crate::native_runs::NativeRunManager::new(crate::native_runs::NativeDeps::new(
        manager.clone(),
        projects.clone(),
        Arc::new(Bus::new()),
        Arc::new(crate::flows::registry::AlwaysAllow),
        Arc::new(crate::models::LocalLock::default()),
        &state_dir,
        None,
    ));
    let launcher = Arc::new(crate::launcher::RunLauncher::new(manager.clone(), native));
    let port = ManagerSessions::new(manager.clone(), projects, launcher);
    let ticket = json!({ "key": MANAGER_TICKET_KEY, "service": project.name });
    assert_eq!(
        port.test_checkout_for(&ticket).is_some(),
        with_test_checkout
    );
    let launch = PipelineLaunch {
        key: MANAGER_TICKET_KEY.to_string(),
        ticket_key: MANAGER_TICKET_KEY.to_string(),
        provider: None,
        permission_mode: None,
        strict_mcp_config: None,
        mcp_config_id: None,
        mcp_servers: None,
        denied_shell_commands: Vec::new(),
        denied_mcp_tools: BTreeMap::new(),
        step: STEP_DEVELOP.to_string(),
        template: "dev-aidp".to_string(),
        project: port.project_for(&ticket).unwrap(),
        inputs_file: workspace.child("inputs.json"),
        auto_approve_gates: None,
        autonomous: false,
        headless: false,
        engine: Some(Engine::Workflow),
        agent_executor: crate::models::enums::agent_executor::AgentExecutor::Internal,
        project_checkout: true,
    };
    let SessionStart::Started(session_id) = port.start(&launch).unwrap() else {
        panic!("expected a started session");
    };
    let cwd = manager.get(&session_id).expect("the session record").cwd;
    wait_for("the session to end", || {
        matches!(port.state(&session_id), SessionState::Ended { .. })
    });
    (cwd, project_dir, clone_dir)
}

fn same_dir(left: &str, right: &Path) -> bool {
    let norm = |text: &str| text.replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(left) == norm(&right.to_string_lossy())
}

#[test]
fn project_flow_session_runs_in_the_test_checkout() {
    let (cwd, project_dir, clone_dir) = project_flow_session_cwd("flows_tc_session", true);
    assert!(same_dir(&cwd, &clone_dir), "cwd {cwd}");
    assert!(!same_dir(&cwd, &project_dir));
}

#[test]
fn project_flow_session_falls_back_to_the_project_without_a_test_checkout() {
    let (cwd, project_dir, _) = project_flow_session_cwd("flows_tc_fallback", false);
    assert!(same_dir(&cwd, &project_dir), "cwd {cwd}");
}

#[test]
fn it_probe_outputs_become_the_step_outputs() {
    let harness = Harness::new("flows_probe_outputs");
    let engine = harness.engine_with_probe(Some(ScriptedProbe { immediate: true }));
    let flow = single_step_flow(wait_step_def(STEP_WATCH, SCRIPTED_PROBE));
    engine.start_with_def(ticket("LOC-20"), flow, None).unwrap();
    wait_step(&engine, "LOC-20", STEP_WATCH, StepStatus::Done);
    let state = engine.get("LOC-20").unwrap();
    assert_eq!(
        state.steps[STEP_WATCH].outputs[OUTPUT_CONCLUSION],
        json!(CONCLUSION_SUCCESS)
    );
}

#[test]
fn it_probe_state_round_trips_between_polls() {
    let harness = Harness::new("flows_probe_state");
    let engine = harness.engine_with_probe(Some(ScriptedProbe { immediate: false }));
    let flow = single_step_flow(wait_step_def(STEP_WATCH, SCRIPTED_PROBE));
    engine.start_with_def(ticket("LOC-21"), flow, None).unwrap();
    wait_step(&engine, "LOC-21", STEP_WATCH, StepStatus::WaitingExternal);

    tick(&engine);
    wait_step(&engine, "LOC-21", STEP_WATCH, StepStatus::Done);
    let state = engine.get("LOC-21").unwrap();
    assert_eq!(
        state.steps[STEP_WATCH].outputs[OUTPUT_SEEN],
        json!({ STATE_POLLS: 1 })
    );
}

/// Runs a writing step carrying one input, returns the holder seen during it and the one left after.
fn holder_around_step(
    tag: &str,
    input: &str,
    value: Value,
    number: u64,
) -> (Option<PrWorkspaceHolder>, Option<PrWorkspaceHolder>) {
    let harness = Harness::new(tag);
    let seen = Arc::new(Mutex::new(None));
    let mut engine_registry = Registry::with_builtins();
    engine_registry.register_action(
        HOLDER_ACTION,
        Arc::new(HolderProbeAction {
            vault: harness.sessions.vault.clone(),
            number,
            seen: seen.clone(),
        }),
    );
    let ports = Ports {
        sessions: harness.sessions.clone(),
        budget: Arc::new(Switch(harness.budget.clone(), harness.ceiling.clone())),
        lock: harness.sessions.lock.clone(),
        bus: harness.bus.clone(),
        clock: harness.clock.clone(),
        ids: Arc::new(CountingIds(AtomicUsize::new(0))),
        runner: Arc::new(ScriptedProcessRunner::new()),
        lock_wait: SHORT_LOCK_WAIT,
    };
    let engine = FlowEngine::new(
        harness.sessions.vault.clone(),
        harness.workspace.child("state"),
        engine_registry,
        ports,
        FlowEngineConfig::default(),
    );
    let step = Step {
        use_: Some(HOLDER_ACTION.to_string()),
        with: BTreeMap::from([(input.to_string(), value)]),
        ..Step::new(STEP_HOLDER, StepKind::Action)
    };
    engine
        .start_with_def(ticket("LOC-90"), single_step_flow(step), None)
        .unwrap();
    wait_for("the run to end", || {
        run_status(&engine, "LOC-90") == FlowStatus::Done
    });
    let after = pr_workspace_holder::read(&harness.sessions.vault, REPO, number);
    let during = seen.lock().unwrap().clone();
    (during, after)
}

/// A string pull request input records the holder and the file goes away with the guard.
#[test]
fn record_holder_writes_for_a_string_input_and_removes_it_on_drop() {
    let (during, after) = holder_around_step(
        "flows_holder_text",
        INPUT_PR_TEXT,
        json!(PR_AS_TEXT),
        PR_AS_TEXT_NUMBER,
    );

    assert_eq!(
        during.map(|holder| holder.stage),
        Some(HOLDER_ACTION.to_string())
    );
    assert_eq!(after, None);
}

/// A numeric pull request input records the holder and the file goes away with the guard.
#[test]
fn record_holder_writes_for_a_numeric_input_and_removes_it_on_drop() {
    let (during, after) = holder_around_step(
        "flows_holder_number",
        INPUT_PR_NUMBER_VALUE,
        json!(PR_AS_NUMBER),
        PR_AS_NUMBER,
    );

    assert_eq!(
        during.map(|holder| holder.stage),
        Some(HOLDER_ACTION.to_string())
    );
    assert_eq!(after, None);
}

/// A busy manual launch writes its refusal and leaves no phantom running session.
#[test]
fn manual_busy_launch_records_the_workspace_refusal() {
    let harness = Harness::new(EXECUTION_KEY);
    let engine = harness.engine();
    let executor = engine.executor();
    let mut state = FlowState::new(EXECUTION_KEY, FlowDef::default());
    state.ticket = ticket(BUSINESS_TICKET_KEY);
    state.status = FlowStatus::Paused;
    state.started_at = Some(executor.now());
    executor
        .store
        .save_state(REPO, &state, &executor.now())
        .unwrap();
    let workspace = workspace_key(&harness.sessions.vault, REPO, EXECUTION_KEY);
    assert!(harness.sessions.lock.try_acquire(&workspace));
    executor
        .start_pipeline(
            REPO,
            &state,
            &aidp_flows::effect_id(
                MANUAL_LABEL,
                0,
                aidp_flows::EffectName::StartPipeline.as_str(),
            ),
            MANUAL_LABEL,
            PIPELINE_TEMPLATE,
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(harness.sessions.launch_count(), 0);
    assert!(executor
        .store
        .load_runtime(REPO, EXECUTION_KEY)
        .sessions
        .is_empty());
    let journal = engine.journal(EXECUTION_KEY).unwrap();
    assert_eq!(
        journal
            .iter()
            .filter(|line| line.line == MANUAL_BUSY_LINE)
            .count(),
        1
    );
    harness.sessions.lock.release(&workspace);
}

/// Retrying a rejected human gate creates a distinct unresolved inbox entry.
#[test]
fn rejected_gate_retry_creates_a_fresh_inbox_entry() {
    let harness = Harness::new(EXECUTION_KEY);
    let engine = harness.engine();
    let mut gate = Step::new(DEPLOY_APPROVAL_GATE, StepKind::Human);
    gate.choices = vec![
        aidp_flows::HumanDecision::Approve.as_str().to_string(),
        aidp_flows::HumanDecision::Reject.as_str().to_string(),
    ];
    engine
        .start_with_def(ticket(EXECUTION_KEY), single_step_flow(gate), None)
        .unwrap();
    engine
        .reject(EXECUTION_KEY, DEPLOY_APPROVAL_GATE, ACTOR)
        .unwrap();
    let rejected = engine.executor().store.load_runtime(REPO, EXECUTION_KEY);
    assert_eq!(rejected.inbox.len(), 1);
    assert!(rejected.inbox[0].resolved_at.is_some());
    engine
        .retry(EXECUTION_KEY, DEPLOY_APPROVAL_GATE, ACTOR)
        .unwrap();
    let retried = engine.executor().store.load_runtime(REPO, EXECUTION_KEY);
    assert_eq!(retried.inbox.len(), 2);
    assert_ne!(retried.inbox[0].id, retried.inbox[1].id);
    assert_eq!(retried.inbox[1].iteration, 1);
    assert!(retried.inbox[1].resolved_at.is_none());
    assert_eq!(harness.sessions.launch_count(), 0);
}

fn first_launch_agent_executor(
    tag: &str,
    policy: crate::models::flow_execution_policy::FlowExecutionPolicy,
) -> crate::models::enums::agent_executor::AgentExecutor {
    let harness = Harness::new(tag);
    let engine = harness.engine();
    let step = Step {
        use_: Some(PIPELINE_TEMPLATE.to_string()),
        ..Step::new(STEP_DEVELOP, StepKind::Pipeline)
    };
    let request = crate::models::TicketFlowRequest {
        ticket: ticket(BUSINESS_TICKET_KEY),
        policy: Some(policy),
        ..Default::default()
    };
    engine
        .start_request_with_def(request, single_step_flow(step))
        .unwrap();
    wait_for("the session", || harness.sessions.launch_count() == 1);
    let executor = harness.sessions.launches.lock().unwrap()[0].agent_executor;
    engine.abort(BUSINESS_TICKET_KEY, ACTOR).unwrap();
    executor
}

#[test]
fn an_external_agents_policy_launches_the_pipeline_with_external_agents() {
    use crate::models::enums::agent_executor::AgentExecutor;
    use crate::models::enums::FlowExecutorMode;
    use crate::models::flow_execution_policy::FlowExecutionPolicy;
    let external = FlowExecutionPolicy {
        executor: FlowExecutorMode::External,
        external_agents: true,
        ..Default::default()
    };
    assert_eq!(
        first_launch_agent_executor("agents_external_launch", external),
        AgentExecutor::External
    );
    assert_eq!(
        first_launch_agent_executor("agents_internal_launch", FlowExecutionPolicy::default()),
        AgentExecutor::Internal
    );
}

#[test]
fn external_agents_without_the_external_executor_refuse_to_start() {
    let harness = Harness::new("agents_need_executor");
    let engine = harness.engine();
    let mut request = execution_request(false, true);
    request.policy.as_mut().unwrap().external_agents = true;

    let error = engine.start_request(request).unwrap_err();

    assert!(error.to_string().contains("externalAgents"), "{error}");
}

#[path = "unity_lock_flow_tests.rs"]
mod unity_lock_flow;

#[test]
fn it_a_ticket_past_its_ceiling_asks_to_continue_and_approval_starts_the_step() {
    let harness = Harness::new("flows_ticket_ceiling");
    harness.ceiling.hit.store(true, Ordering::SeqCst);
    let engine = harness.engine();
    engine
        .start(ticket("LOC-15"), Some("test-flow"), None)
        .unwrap();

    wait_step(&engine, "LOC-15", STEP_DEVELOP, StepStatus::WaitingHuman);
    let state = engine.get("LOC-15").unwrap();
    assert_eq!(
        state.steps[STEP_DEVELOP].reason.as_deref(),
        Some("ticket-budget")
    );
    assert_eq!(harness.sessions.launch_count(), 0);
    tick(&engine);
    wait_for("the inbox entry", || engine.inbox_entries().len() == 1);

    engine.approve("LOC-15", STEP_DEVELOP, ACTOR).unwrap();

    assert_eq!(harness.ceiling.raised.load(Ordering::SeqCst), 1);
    wait_for("the launch", || harness.sessions.launch_count() == 1);
}

#[test]
fn it_rejecting_the_ceiling_escalates_the_run() {
    let harness = Harness::new("flows_ticket_ceiling_reject");
    harness.ceiling.hit.store(true, Ordering::SeqCst);
    let engine = harness.engine();
    engine
        .start(ticket("LOC-16"), Some("test-flow"), None)
        .unwrap();
    wait_step(&engine, "LOC-16", STEP_DEVELOP, StepStatus::WaitingHuman);

    engine.reject("LOC-16", STEP_DEVELOP, ACTOR).unwrap();

    assert_eq!(run_status(&engine, "LOC-16"), FlowStatus::Escalated);
    assert_eq!(harness.ceiling.raised.load(Ordering::SeqCst), 0);
    assert_eq!(harness.sessions.launch_count(), 0);
}

#[test]
fn it_a_failed_action_reports_a_truncated_stderr_not_the_generic_message() {
    let long = "x".repeat(5000);
    let ActionOutcome::TechnicalError(message) = classify_exit(1, "", &long, &[]) else {
        panic!("technical error expected");
    };
    assert!(message.starts_with("exit 1: xxx"), "{message}");
    assert!(message.len() < 2100, "{}", message.len());
    let ActionOutcome::TechnicalError(message) =
        classify_exit(1, "", "fatal: Unable to create index.lock", &[])
    else {
        panic!("technical error expected");
    };
    assert_eq!(message, "exit 1: fatal: Unable to create index.lock");
}

fn run_with_merge_outputs(merged: bool, conflict: bool) -> FlowState {
    let mut state = FlowState::new(MERGE_TICKET, FlowDef::default());
    let merge = aidp_flows::StepState {
        status: StepStatus::Done,
        outputs: BTreeMap::from([
            (MERGE_OUTPUT_MERGED.to_string(), json!(merged)),
            (MERGE_OUTPUT_CONFLICT.to_string(), json!(conflict)),
        ]),
        ..aidp_flows::StepState::default()
    };
    state.steps.insert(STEP_MERGE_ID.to_string(), merge);
    state
}

/// A merge step ended on a base conflict does not make the ticket done: the flow resolves it.
#[test]
fn a_merge_step_ended_on_a_conflict_keeps_the_ticket_open() {
    assert_eq!(
        ticket_status_of(&run_with_merge_outputs(false, true)),
        Some(aidp_tickets::model::Status::InProgress)
    );
    assert_eq!(
        ticket_status_of(&run_with_merge_outputs(true, false)),
        Some(aidp_tickets::model::Status::Done)
    );
}

#[test]
fn it_a_busy_workspace_is_journaled_once_not_at_every_retry() {
    let harness = Harness::new("flows_busy_journal");
    let held = workspace_key(&harness.sessions.vault, REPO, "LOC-30");
    assert!(harness.sessions.lock.try_acquire(&held));
    let engine = harness.engine();
    engine
        .start(ticket("LOC-30"), Some("test-flow"), None)
        .unwrap();
    for _ in 0..3 {
        tick(&engine);
    }
    let waiting = |engine: &FlowEngine| {
        engine
            .journal("LOC-30")
            .unwrap()
            .iter()
            .filter(|line| line.line.contains("waiting to start: workspace busy"))
            .count()
    };
    wait_for("the busy line", || waiting(&engine) >= 1);
    for _ in 0..3 {
        tick(&engine);
    }
    assert_eq!(waiting(&engine), 1);
    assert_eq!(harness.sessions.launch_count(), 0);
}

#[test]
fn it_a_worktree_flow_pipeline_denies_the_unity_mcp() {
    let harness = Harness::new("flows_deny_unity_mcp");
    let engine = harness.engine();
    engine
        .start(ticket("LOC-31"), Some("test-flow"), None)
        .unwrap();
    wait_for("the session", || harness.sessions.launch_count() == 1);
    let launches = harness.sessions.launches.lock().unwrap();
    assert!(!launches[0].project_checkout);
    assert_eq!(
        launches[0].denied_mcp_tools.get("unity-mcp"),
        Some(&vec!["*".to_string()])
    );
}

/// A `git.restore` that fails `failures` times, then works.
struct FlakyRestore {
    failures: AtomicUsize,
    runs: AtomicUsize,
}

impl FlowAction for FlakyRestore {
    fn writes(&self) -> bool {
        false
    }

    fn run(&self, _context: &ActionContext<'_>, _with: &BTreeMap<String, Value>) -> ActionOutcome {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let left = self.failures.load(Ordering::SeqCst);
        if left > 0 {
            self.failures.store(left - 1, Ordering::SeqCst);
            return ActionOutcome::TechnicalError("git restore failed".to_string());
        }
        ActionOutcome::Ok(BTreeMap::new())
    }
}

#[test]
fn it_a_failed_end_restore_is_journaled_once_held_in_the_test_lane_and_retried() {
    let harness = Harness::new("flows_restore_retry");
    let restore = Arc::new(FlakyRestore {
        failures: AtomicUsize::new(2),
        runs: AtomicUsize::new(0),
    });
    *harness.restore.lock().unwrap() = Some(restore.clone());
    let engine = harness.engine();
    let flow = FlowDef {
        id: "inline".to_string(),
        version: 1,
        workspace: Some(aidp_flows::models::enums::WorkspaceMode::Project),
        steps: vec![
            action_step("count", COUNT_ACTION),
            Step::new("gate", StepKind::Human),
        ],
        ..FlowDef::default()
    };
    engine.start_with_def(ticket("LOC-32"), flow, None).unwrap();
    wait_step(&engine, "LOC-32", "gate", StepStatus::WaitingHuman);
    assert_eq!(engine.lane_load().1.values().sum::<usize>(), 1);

    engine.abort("LOC-32", ACTOR).unwrap();
    wait_for("the failed restore", || {
        !engine.executor().failed_restores().is_empty()
    });
    let failures = |engine: &FlowEngine| {
        engine
            .journal("LOC-32")
            .unwrap()
            .iter()
            .filter(|line| line.line.contains("git.restore failed"))
            .count()
    };
    assert_eq!(failures(&engine), 1);
    // The ended run still occupies the test checkout.
    assert_eq!(engine.lane_load().1.values().sum::<usize>(), 1);

    wait_for("the restore to work", || {
        engine.retry_failed_restores();
        engine.executor().failed_restores().is_empty()
    });
    assert_eq!(restore.runs.load(Ordering::SeqCst), 3);
    assert_eq!(failures(&engine), 1, "journaled once");
    assert_eq!(engine.lane_load().1.values().sum::<usize>(), 0);
}

#[test]
fn it_an_aborted_run_forgets_its_start_wait_reason() {
    let harness = Harness::new("flows_busy_forget");
    let held = workspace_key(&harness.sessions.vault, REPO, "LOC-33");
    assert!(harness.sessions.lock.try_acquire(&held));
    let engine = harness.engine();
    engine
        .start(ticket("LOC-33"), Some("test-flow"), None)
        .unwrap();
    tick(&engine);
    wait_for("the busy line", || {
        engine
            .journal("LOC-33")
            .unwrap()
            .iter()
            .any(|line| line.line.contains("waiting to start"))
    });
    engine.abort("LOC-33", ACTOR).unwrap();
    assert!(!engine.executor().has_start_wait("LOC-33"));
}

/// A `git.restore` that takes a while.
struct SlowRestore(AtomicUsize);

impl FlowAction for SlowRestore {
    fn writes(&self) -> bool {
        false
    }

    fn run(&self, _context: &ActionContext<'_>, _with: &BTreeMap<String, Value>) -> ActionOutcome {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(600));
        ActionOutcome::TechnicalError("slow failure".to_string())
    }
}

#[test]
fn it_a_slow_restore_is_not_respawned_by_every_retry() {
    let harness = Harness::new("flows_restore_inflight");
    let slow = Arc::new(SlowRestore(AtomicUsize::new(0)));
    *harness.restore.lock().unwrap() = Some(slow.clone());
    let engine = harness.engine();
    let flow = FlowDef {
        id: "inline".to_string(),
        version: 1,
        workspace: Some(aidp_flows::models::enums::WorkspaceMode::Project),
        steps: vec![
            action_step("count", COUNT_ACTION),
            Step::new("gate", StepKind::Human),
        ],
        ..FlowDef::default()
    };
    engine.start_with_def(ticket("LOC-34"), flow, None).unwrap();
    wait_step(&engine, "LOC-34", "gate", StepStatus::WaitingHuman);
    engine.abort("LOC-34", ACTOR).unwrap();
    wait_for("the first failure", || {
        !engine.executor().failed_restores().is_empty()
    });
    wait_for("the retry to start", || {
        engine.retry_failed_restores();
        slow.0.load(Ordering::SeqCst) == 2
    });
    for _ in 0..5 {
        engine.retry_failed_restores();
    }
    assert_eq!(slow.0.load(Ordering::SeqCst), 2);
}
