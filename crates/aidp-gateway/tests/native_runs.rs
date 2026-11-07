//! End-to-end native runs: the daemon drives whole pipelines through a scripted provider.

#[path = "common_native/fake_provider.rs"]
#[allow(dead_code)]
pub mod fake_provider;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aidp_agents::{
    AgentError, AgentHandle, AgentProvider, AgentSpec, Capabilities, ProvidersConfig,
};
use aidp_core::models::escalation::Escalation;
use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner};
use aidp_gateway::bus::Bus;
use aidp_gateway::flows::registry::AlwaysAllow;
use aidp_gateway::models::enums::native_run_status::NativeRunStatus;
use aidp_gateway::models::enums::native_stage_status::NativeStageStatus;
use aidp_gateway::models::enums::{PermissionMode, SessionStatus};
use aidp_gateway::models::native_run_start::StartNativeRun;
use aidp_gateway::models::native_run_view::NativeRunView;
use aidp_gateway::models::LocalLock;
use aidp_gateway::native_runs::{http_routes, NativeDeps, NativeRunManager};
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::SessionManager;
use aidp_testkit::{EnvGuard, TempWorkspace};
use fake_provider::{Behaviour, Call, Refusal, ScriptedProvider, BLOCKING_SIGNAL, PROVIDER_ID};
use serde_json::Value;
use tiny_http::Method;

const DEV_EXEMPLE_TEMPLATE_ID: &str = "dev-exemple";
const FIXTURES_DIR: &str = "tests/fixtures/native_runs";
const CONDITIONS_TEMPLATE: &str = "conditions_loop.yaml";
const GATED_TEMPLATE: &str = "gated.yaml";
const SCOPE_TEMPLATE: &str = "out_of_scope.yaml";
const PLAN_JSON: &str = include_str!("fixtures/native_runs/dev_exemple_plan.json");
const VAULT_ENV: &str = "AIDP_VAULT";
const HOME_ENV: &str = "HOME";
const USER_PROFILE_ENV: &str = "USERPROFILE";
const FEATURE_INPUT: &str = "feature";
const PROMPT_INPUT: &str = "prompt";
const ALL_GATES: &str = "*";
const GATE_STAGE: &str = "ship";
const SHUTDOWN_TEST_TAG: &str = "native-shutdown-gate";
const SHUTDOWN_STATUS: u16 = 503;
const DRAFT_STAGE: &str = "draft";
const FINISH_STAGE: &str = "finish";
const AUTONOMY_FIXTURES_DIR: &str = "tests/fixtures/autonomy";
const AUTONOMOUS_TEMPLATE: &str = "autonomous.yaml";
const BLOCKED_REPLY: &str = include_str!("fixtures/autonomy/blocked_reply.json");
const ESCALATION_REPLY: &str = include_str!("fixtures/autonomy/escalation_reply.json");
const THREE_THREADS_REPLY: &str = include_str!("fixtures/autonomy/three_threads_reply.json");
const ASK_STAGE: &str = "ask";
const AFTER_STAGE: &str = "after";
const AUTONOMOUS_ENV: &str = "AIDP_AUTONOMOUS";
const AUTONOMOUS_ENABLED: &str = "1";
const DISALLOWED_TOOLS_FLAG: &str = "--disallowedTools";
const ADD_DIR_FLAG: &str = "--add-dir";
const BYPASS_MODE: &str = "bypassPermissions";
const AUTO_MODE: &str = "auto";
const DEFAULT_MODE: &str = "default";
const EXPLICIT_PERMISSIONS_TEST: &str = "native-permissions-explicit";
const REFUSED_OVERRIDE_TEST: &str = "native-permissions-explicit-refused";
const MODEL_OVERRIDE_TEST: &str = "native-model-effort-overrides";
const HEADLESS_TEST: &str = "native-permissions-headless";
const DONT_ASK_MODE: &str = "dontAsk";
const MALFORMED_REPLY: &str = "not json";
const FAILED_DRAFT_SESSIONS: usize = 2;
const SESSIONS_FROM_THE_RETRY: usize = 3;
const MODEL_OVERRIDE: &str = "custom-native-model";
const EFFORT_OVERRIDE: &str = "high";
const LEGACY_PIPELINES_DIR: &str = ".aidp/pipelines";
const REPLY_REJECTED: &str = "reply rejected after one repair";
const ASK_USER_QUESTION_TOOL: &str = "AskUserQuestion";
const QUESTION_ID: &str = "q1";
const QUESTION_TEXT: &str = "Which database?";
const RECOMMENDED_CHOICE: &str = "Postgres";
const RECOMMENDED_REASON: &str = "recommended";
const BLOCKED_TWICE_REASON: &str = "blocked-twice";
const RETRY_HEADING: &str = "Autonomous retry";
const RUN_STATE_FILE: &str = "run-state.json";
const MODE_KEY: &str = "mode";
const NATIVE_MODE: &str = "native";
const ORIGINAL_README: &str = "original\n";
const README: &str = "README.md";
const FORBIDDEN_FILE: &str = "forbidden.txt";
const ALLOWED_FILE: &str = "allowed/ok.txt";
const KEEP_FILE: &str = "allowed/keep.txt";
const SETTLE: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(20);
const RUN_POLL: Duration = Duration::from_millis(10);
const APPROVE_PATH_PREFIX: &str = "/api/runs/";
const OK_STATUS: u16 = 200;
const EXTRA_STATE_KEYS: &[&str] = &[
    "mode",
    "cost",
    "plan_schedules",
    "loops",
    "approved_gates",
    "pending_gate",
    "workflow_run_id",
    "last_workflow_status",
    "composites_crossed",
    "crossed_composites",
];
const DEV_EXEMPLE_LABELS: &[&str] = &[
    "collect",
    "plan",
    "implementation#T1",
    "implementation#T2",
    "implementation",
];
const CLIENT_REQUEST_ID: &str = "ada-run-91";

#[test]
fn client_request_id_reuses_one_run_concurrently_after_restart_and_completion() {
    let harness = plain_harness(CLIENT_REQUEST_ID, |_| Behaviour::ok());
    let mut request = harness.request(&fixture(GATED_TEMPLATE), &[]);
    request.client_request_id = Some(CLIENT_REQUEST_ID.to_string());
    let run_ids = std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..6)
            .map(|_| scope.spawn(|| harness.stack.manager.start(request.clone()).unwrap()))
            .collect();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(run_ids.iter().all(|run_id| run_id == &run_ids[0]));
    assert_eq!(harness.stack.manager.list().len(), 1);
    let run_id = &run_ids[0];
    assert_eq!(harness.finish(run_id).status, NativeRunStatus::WaitingGate);
    harness.stack.manager.approve(run_id, GATE_STAGE).unwrap();
    assert_eq!(harness.finish(run_id).status, NativeRunStatus::Done);
    let restarted = harness.restarted_stack();
    assert_eq!(restarted.manager.start(request.clone()).unwrap(), *run_id);
    let mut different = request;
    different.autonomous = true;
    let error = restarted.manager.start(different).unwrap_err();
    assert_eq!(
        aidp_gateway::models::ApiError::from(&error).status.status(),
        aidp_gateway::models::enums::ApiStatus::Conflict.status()
    );
    assert_eq!(restarted.manager.list().len(), 1);
}

/// Records the environment and launch tail of every session, then delegates to the scripted provider.
struct SpyProvider {
    inner: Arc<ScriptedProvider>,
    specs: Mutex<Vec<AgentSpec>>,
}

impl AgentProvider for SpyProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn start(&self, spec: AgentSpec) -> Result<AgentHandle, AgentError> {
        self.specs.lock().unwrap().push(spec.clone());
        self.inner.start(spec)
    }
}

struct Stack {
    sessions: Arc<SessionManager>,
    manager: Arc<NativeRunManager>,
}

struct Harness {
    provider: Arc<ScriptedProvider>,
    spy: Arc<SpyProvider>,
    projects: Arc<Projects>,
    bus: Arc<Bus>,
    project_id: String,
    state_dir: PathBuf,
    workspace: PathBuf,
    stack: Stack,
    _root: TempWorkspace,
    _env: EnvGuard,
}

fn run_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_workspace(directory: &Path) {
    std::fs::create_dir_all(directory.join("allowed")).unwrap();
    std::fs::write(directory.join(README), ORIGINAL_README).unwrap();
    std::fs::write(directory.join(KEEP_FILE), "keep\n").unwrap();
    run_git(directory, &["init", "-q"]);
    run_git(directory, &["config", "user.email", "native@example.test"]);
    run_git(directory, &["config", "user.name", "native"]);
    run_git(directory, &["config", "core.autocrlf", "false"]);
    run_git(directory, &["add", "."]);
    run_git(directory, &["commit", "-q", "-m", "initial"]);
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURES_DIR)
        .join(name)
}

fn autonomy_template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(AUTONOMY_FIXTURES_DIR)
        .join(AUTONOMOUS_TEMPLATE)
}

fn dev_exemple_template() -> PathBuf {
    aidp_testkit::fixtures::template(DEV_EXEMPLE_TEMPLATE_ID)
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

fn build_stack(
    state_dir: &Path,
    bus: &Arc<Bus>,
    projects: &Arc<Projects>,
    provider: &Arc<SpyProvider>,
) -> Stack {
    let runner: Arc<dyn ProcessRunner> = Arc::new(ScriptedProcessRunner::new());
    let sessions =
        SessionManager::load_with_runner(state_dir, bus.clone(), projects.clone(), runner)
            .with_provider(PROVIDER_ID, provider.clone())
            .with_providers_config(ProvidersConfig::default());
    let mut deps = NativeDeps::new(
        sessions.clone(),
        projects.clone(),
        bus.clone(),
        Arc::new(AlwaysAllow),
        Arc::new(LocalLock::default()),
        state_dir,
        None,
    );
    deps.poll_interval = RUN_POLL;
    Stack {
        sessions,
        manager: NativeRunManager::new(deps),
    }
}

impl Harness {
    fn new(tag: &str, provider: Arc<ScriptedProvider>) -> Harness {
        let root = TempWorkspace::new(tag);
        let vault = root.child("vault");
        let home = root.child("home");
        let workspace = root.child("workspace");
        let state_dir = root.child("state");
        for directory in [&vault, &home, &state_dir] {
            std::fs::create_dir_all(directory).unwrap();
        }
        init_workspace(&workspace);
        let env = EnvGuard::with(&[
            (VAULT_ENV, Some(vault.to_str().unwrap())),
            (HOME_ENV, Some(home.to_str().unwrap())),
            (USER_PROFILE_ENV, Some(home.to_str().unwrap())),
        ]);
        let projects = Arc::new(Projects::load(&state_dir));
        let project_id = projects.add(workspace.to_str().unwrap()).id;
        let bus = Arc::new(Bus::new());
        let spy = Arc::new(SpyProvider {
            inner: provider.clone(),
            specs: Mutex::new(Vec::new()),
        });
        let stack = build_stack(&state_dir, &bus, &projects, &spy);
        Harness {
            provider,
            spy,
            projects,
            bus,
            project_id,
            state_dir,
            workspace,
            stack,
            _root: root,
            _env: env,
        }
    }

    fn restarted_stack(&self) -> Stack {
        build_stack(&self.state_dir, &self.bus, &self.projects, &self.spy)
    }

    fn request(&self, template: &Path, auto_approve: &[&str]) -> StartNativeRun {
        StartNativeRun {
            project_id: self.project_id.clone(),
            template_id: template.to_string_lossy().to_string(),
            inputs: run_inputs(),
            provider: Some(PROVIDER_ID.to_string()),
            auto_approve_gates: auto_approve.iter().map(|gate| gate.to_string()).collect(),
            ..StartNativeRun::default()
        }
    }

    fn start_autonomous(&self, template: &Path) -> String {
        let request = StartNativeRun {
            autonomous: true,
            ..self.request(template, &[])
        };
        self.stack
            .manager
            .start(request)
            .expect("the autonomous run starts")
    }

    fn start(&self, template: &Path, auto_approve: &[&str]) -> String {
        self.stack
            .manager
            .start(self.request(template, auto_approve))
            .expect("the run starts")
    }

    fn view(&self, run_id: &str) -> NativeRunView {
        self.stack.manager.get(run_id).expect("the run exists")
    }

    fn finish(&self, run_id: &str) -> NativeRunView {
        self.stack
            .manager
            .wait_for(run_id, SETTLE)
            .expect("the run exists")
    }

    fn wait_for_status(&self, run_id: &str, status: NativeRunStatus) -> NativeRunView {
        wait_until("the run status", || self.view(run_id).status == status);
        self.view(run_id)
    }

    fn wait_for_session(&self, run_id: &str, label: &str) {
        wait_until("a running session", || {
            self.view(run_id).stage(label).is_some_and(|row| {
                row.status == NativeStageStatus::Running && !row.session_ids.is_empty()
            })
        });
    }

    fn state_json(&self, run_id: &str) -> Value {
        let file = self.view(run_id).run_dir.join(RUN_STATE_FILE);
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
    }
}

fn run_inputs() -> BTreeMap<String, Value> {
    BTreeMap::from([
        (FEATURE_INPUT.to_string(), Value::from("native-feature")),
        (PROMPT_INPUT.to_string(), Value::from("say hello")),
    ])
}

fn dev_exemple_behaviour(call: &Call) -> Behaviour {
    match call.label.as_str() {
        "implementation#T1" => Behaviour::ok().write("src/greeting.txt", "hello\n"),
        "implementation#T2" => Behaviour::ok().write("tests/greeting_check.txt", "hello\n"),
        _ => Behaviour::ok(),
    }
}

fn dev_exemple_harness(tag: &str) -> Harness {
    Harness::new(
        tag,
        ScriptedProvider::new(Some(PLAN_JSON), dev_exemple_behaviour),
    )
}

fn plain_harness(
    tag: &str,
    decide: impl Fn(&Call) -> Behaviour + Send + Sync + 'static,
) -> Harness {
    Harness::new(tag, ScriptedProvider::new(None, decide))
}

fn run_dev_exemple(harness: &Harness) -> String {
    let run_id = harness.start(&dev_exemple_template(), &[]);
    let view = harness.finish(&run_id);
    assert_done(&view);
    run_id
}

fn state_keys(state: &Value) -> BTreeSet<String> {
    state.as_object().unwrap().keys().cloned().collect()
}

fn workflow_state(harness: &Harness) -> Value {
    let vault = aidp_core::vault::resolve_global();
    let state = aidp_core::run::init(
        &dev_exemple_template(),
        &harness.workspace,
        serde_json::to_value(run_inputs()).unwrap(),
        vault.as_ref(),
    )
    .expect("workflow-mode init");
    let text = std::fs::read_to_string(state.run_dir.join(RUN_STATE_FILE)).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn row_status(view: &NativeRunView, name: &str) -> NativeStageStatus {
    view.stage(name)
        .unwrap_or_else(|| panic!("no row {name}"))
        .status
}

fn describe(view: &NativeRunView) -> String {
    let rows: Vec<String> = view
        .stages
        .iter()
        .map(|row| format!("{}={:?}({:?})", row.name, row.status, row.reason))
        .collect();
    format!(
        "{:?} {:?} rows: {}",
        view.status,
        view.reason,
        rows.join(", ")
    )
}

fn assert_done(view: &NativeRunView) {
    assert_eq!(view.status, NativeRunStatus::Done, "{}", describe(view));
}

#[test]
fn ac1_dev_exemple_runs_one_session_per_plan_task_in_order() {
    let harness = dev_exemple_harness("native-ac1");
    let run_id = run_dev_exemple(&harness);
    let view = harness.view(&run_id);

    assert_eq!(harness.provider.labels(), DEV_EXEMPLE_LABELS);
    for name in DEV_EXEMPLE_LABELS {
        assert_eq!(
            row_status(&view, name),
            NativeStageStatus::Done,
            "row {name}"
        );
    }
    assert!(harness.workspace.join("src/greeting.txt").is_file());
    assert!(harness.workspace.join("tests/greeting_check.txt").is_file());
}

#[test]
fn ac1_native_run_state_has_the_keys_of_a_workflow_run_state() {
    let harness = dev_exemple_harness("native-ac1-keys");
    let run_id = run_dev_exemple(&harness);
    let native = harness.state_json(&run_id);
    let workflow = workflow_state(&harness);

    assert_eq!(native[MODE_KEY], NATIVE_MODE);
    assert!(workflow.get(MODE_KEY).is_none());
    let native_keys = state_keys(&native);
    let workflow_keys = state_keys(&workflow);
    let missing: Vec<_> = workflow_keys.difference(&native_keys).collect();
    assert!(missing.is_empty(), "keys lost in native mode: {missing:?}");
    let unexpected: Vec<_> = native_keys
        .difference(&workflow_keys)
        .filter(|key| !EXTRA_STATE_KEYS.contains(&key.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "unexpected native keys: {unexpected:?}"
    );
}

#[test]
fn ac2_invalid_reply_is_repaired_by_one_relaunch() {
    let harness = plain_harness("native-ac2-repair", |call| {
        if call.label == "draft" && call.nth == 1 {
            Behaviour::raw("this is not json")
        } else {
            Behaviour::ok()
        }
    });
    let run_id = harness.start(&fixture(GATED_TEMPLATE), &[ALL_GATES]);
    let view = harness.finish(&run_id);

    assert_done(&view);
    assert_eq!(harness.provider.count("draft"), 2);
    assert_eq!(view.stage("draft").unwrap().attempts, 2);
    assert!(harness.provider.calls()[1].prompt.contains("rejected"));
}

#[test]
fn ac2_second_invalid_reply_fails_the_stage_with_a_reason() {
    let harness = plain_harness("native-ac2-fail", |call| {
        if call.label == "draft" {
            Behaviour::raw("still not json")
        } else {
            Behaviour::ok()
        }
    });
    let run_id = harness.start(&fixture(GATED_TEMPLATE), &[ALL_GATES]);
    let view = harness.finish(&run_id);

    assert_eq!(view.status, NativeRunStatus::Paused);
    let row = view.stage("draft").unwrap();
    assert_eq!(row.status, NativeStageStatus::Failed);
    assert!(row
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("reply rejected after one repair"));
    assert_eq!(harness.provider.count("draft"), 2);
    assert_eq!(harness.provider.count(GATE_STAGE), 0);
}

fn loop_behaviour(
    initially_blocking: bool,
    fix_converges: bool,
) -> impl Fn(&Call) -> Behaviour + Send + Sync + 'static {
    move |call| match call.label.as_str() {
        "review" => Behaviour::ok().signal(BLOCKING_SIGNAL, Value::Bool(initially_blocking)),
        "rereview" => Behaviour::ok().signal(BLOCKING_SIGNAL, Value::Bool(!fix_converges)),
        _ => Behaviour::ok(),
    }
}

#[test]
fn ac3_false_conditions_skip_stages_and_the_loop_stops_at_its_condition() {
    let harness = plain_harness("native-ac3-converge", loop_behaviour(true, true));
    let run_id = harness.start(&fixture(CONDITIONS_TEMPLATE), &[]);
    let view = harness.finish(&run_id);

    assert_done(&view);
    for skipped in ["static_off", "deep_scan"] {
        let row = view.stage(skipped).unwrap();
        assert_eq!(row.status, NativeStageStatus::Skipped, "row {skipped}");
        assert!(!row.reason.as_deref().unwrap_or_default().is_empty());
        assert_eq!(harness.provider.count(skipped), 0);
    }
    assert_eq!(harness.provider.count("refix"), 1);
    assert_eq!(harness.provider.count("rereview"), 1);
    assert_eq!(
        row_status(&view, "correction_loop"),
        NativeStageStatus::Done
    );
}

#[test]
fn ac3_loop_without_blocking_findings_never_enters() {
    let harness = plain_harness("native-ac3-clean", loop_behaviour(false, true));
    let run_id = harness.start(&fixture(CONDITIONS_TEMPLATE), &[]);
    let view = harness.finish(&run_id);

    assert_done(&view);
    assert_eq!(harness.provider.count("refix"), 0);
}

#[test]
fn ac3_loop_stops_at_max_iterations_and_pauses() {
    let harness = plain_harness("native-ac3-max", loop_behaviour(true, false));
    let run_id = harness.start(&fixture(CONDITIONS_TEMPLATE), &[]);
    let view = harness.finish(&run_id);

    assert_eq!(view.status, NativeRunStatus::Paused);
    assert!(view
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("did not converge"));
    assert_eq!(harness.provider.count("refix"), 2);
    assert_eq!(harness.provider.count("rereview"), 2);
}

#[test]
fn ac4_gate_waits_then_resumes_on_approval() {
    let harness = plain_harness("native-ac4-gate", |_| Behaviour::ok());
    let run_id = harness.start(&fixture(GATED_TEMPLATE), &[]);
    let waiting = harness.wait_for_status(&run_id, NativeRunStatus::WaitingGate);

    assert_eq!(waiting.pending_gate.as_deref(), Some(GATE_STAGE));
    assert_eq!(row_status(&waiting, "draft"), NativeStageStatus::Done);
    assert_eq!(harness.provider.count(GATE_STAGE), 0);

    harness.stack.manager.approve(&run_id, GATE_STAGE).unwrap();
    let view = harness.finish(&run_id);
    assert_done(&view);
    assert_eq!(harness.provider.count(GATE_STAGE), 1);
    assert_eq!(harness.provider.count("finish"), 1);
    assert!(view.pending_gate.is_none());
}

#[test]
fn ac4_gate_is_approved_through_the_http_route() {
    let harness = plain_harness("native-ac4-http", |_| Behaviour::ok());
    let run_id = harness.start(&fixture(GATED_TEMPLATE), &[]);
    harness.wait_for_status(&run_id, NativeRunStatus::WaitingGate);

    let path = format!("{APPROVE_PATH_PREFIX}{run_id}/stages/{GATE_STAGE}/approve");
    let reply = http_routes::handle(&harness.stack.manager, &Method::Post, &path, &Value::Null)
        .expect("the route exists");
    assert_eq!(reply.status, OK_STATUS);

    let view = harness.finish(&run_id);
    assert_done(&view);
    assert_eq!(harness.provider.count(GATE_STAGE), 1);
}

#[test]
fn ac4_auto_approved_gates_are_traversed() {
    for auto in [vec![GATE_STAGE], vec![ALL_GATES]] {
        let harness = plain_harness("native-ac4-auto", |_| Behaviour::ok());
        let run_id = harness.start(&fixture(GATED_TEMPLATE), &auto);
        let view = harness.finish(&run_id);

        assert_done(&view);
        assert!(view.pending_gate.is_none());
        assert_eq!(harness.provider.count("finish"), 1);
    }
}

#[test]
fn shutdown_joins_gated_executors_and_preserves_restart_state() {
    let harness = plain_harness(SHUTDOWN_TEST_TAG, |_| Behaviour::ok());
    let run_id = harness.start(&fixture(GATED_TEMPLATE), &[]);
    harness.wait_for_status(&run_id, NativeRunStatus::WaitingGate);
    let before = harness.state_json(&run_id);
    let stopped_at = Instant::now();
    harness.stack.manager.shutdown();
    assert!(stopped_at.elapsed() < SETTLE);
    assert_eq!(harness.state_json(&run_id), before);
    assert_eq!(harness.view(&run_id).status, NativeRunStatus::WaitingGate);
    let reply = http_routes::handle(
        &harness.stack.manager,
        &Method::Post,
        &format!("{APPROVE_PATH_PREFIX}{run_id}/stages/{GATE_STAGE}/approve"),
        &Value::Null,
    )
    .unwrap();
    assert_eq!(reply.status, SHUTDOWN_STATUS);
    assert!(harness
        .stack
        .manager
        .start(harness.request(&fixture(GATED_TEMPLATE), &[]))
        .is_err());
    assert_eq!(harness.stack.manager.resume_all(), 0);
    let restarted = harness.restarted_stack();
    assert_eq!(restarted.manager.resume_all(), 1);
    wait_until(GATE_STAGE, || {
        restarted.manager.get(&run_id).unwrap().status == NativeRunStatus::WaitingGate
    });
    restarted.manager.approve(&run_id, GATE_STAGE).unwrap();
    let completed = restarted.manager.wait_for(&run_id, SETTLE).unwrap();
    assert_done(&completed);
    assert_eq!(harness.provider.count(DRAFT_STAGE), 1);
    assert_eq!(harness.provider.count(FINISH_STAGE), 1);
    restarted.manager.shutdown();
}

#[test]
fn ac5_restart_relaunches_the_running_stage_once_and_replays_nothing() {
    let harness = plain_harness("native-ac5", |call| {
        if call.label == "finish" && call.nth == 1 {
            Behaviour::hang()
        } else {
            Behaviour::ok()
        }
    });
    let run_id = harness.start(&fixture(GATED_TEMPLATE), &[ALL_GATES]);
    harness.wait_for_session(&run_id, "finish");
    let interrupted = harness.view(&run_id).stage("finish").unwrap().session_ids[0].clone();

    let restarted = harness.restarted_stack();
    assert_eq!(restarted.manager.resume_all(), 1);
    let view = restarted
        .manager
        .wait_for(&run_id, SETTLE)
        .expect("the run is known after the restart");

    assert_done(&view);
    assert_eq!(harness.provider.count("draft"), 1);
    assert_eq!(harness.provider.count(GATE_STAGE), 1);
    assert_eq!(harness.provider.count("finish"), 2);
    let relaunch = &harness.provider.calls()[3];
    assert_eq!(relaunch.label, "finish");
    assert!(relaunch.prompt.contains("Resumption"));
    let finish = view.stage("finish").unwrap();
    assert!(finish.resumed);
    assert_eq!(finish.attempts, 2);
    assert_eq!(
        restarted.sessions.get(&interrupted).unwrap().status,
        SessionStatus::Orphaned
    );
}

#[test]
fn ac6_cancel_stops_every_session_and_marks_the_run_cancelled() {
    let harness = plain_harness("native-ac6", |call| {
        if call.label == "draft" {
            Behaviour::hang()
        } else {
            Behaviour::ok()
        }
    });
    let run_id = harness.start(&fixture(GATED_TEMPLATE), &[ALL_GATES]);
    harness.wait_for_session(&run_id, "draft");

    harness.stack.manager.cancel(&run_id).unwrap();
    let view = harness.finish(&run_id);

    assert_eq!(view.status, NativeRunStatus::Cancelled);
    assert_eq!(row_status(&view, "draft"), NativeStageStatus::Cancelled);
    assert_eq!(harness.provider.count(GATE_STAGE), 0);
    let session_ids = view.session_ids();
    assert!(!session_ids.is_empty());
    wait_until("the sessions to stop", || {
        session_ids.iter().all(|id| {
            harness
                .stack
                .sessions
                .get(id)
                .is_some_and(|record| record.status == SessionStatus::Cancelled)
        })
    });
}

fn read_workspace_file(harness: &Harness, relative_path: &str) -> String {
    std::fs::read_to_string(harness.workspace.join(relative_path))
        .unwrap()
        .replace("\r\n", "\n")
}

#[test]
fn ac7_out_of_scope_writes_are_restored_and_fail_the_stage() {
    let harness = plain_harness("native-ac7", |_| {
        Behaviour::ok()
            .write(ALLOWED_FILE, "inside\n")
            .write(FORBIDDEN_FILE, "outside\n")
            .write(README, "tampered\n")
    });
    let run_id = harness.start(&fixture(SCOPE_TEMPLATE), &[]);
    let view = harness.finish(&run_id);

    assert_eq!(view.status, NativeRunStatus::Paused);
    let row = view.stage("edit").unwrap();
    assert_eq!(row.status, NativeStageStatus::Failed);
    let reason = row.reason.clone().unwrap_or_default();
    assert!(reason.contains("outside its scope"), "reason: {reason}");
    assert!(reason.contains(FORBIDDEN_FILE) && reason.contains(README));
    assert!(!harness.workspace.join(FORBIDDEN_FILE).exists());
    assert_eq!(read_workspace_file(&harness, README), ORIGINAL_README);
    assert_eq!(read_workspace_file(&harness, ALLOWED_FILE), "inside\n");
}

#[test]
fn ac8_every_stage_carries_its_cost_in_run_state() {
    let harness = dev_exemple_harness("native-ac8");
    let run_id = run_dev_exemple(&harness);
    let state = harness.state_json(&run_id);

    let stages = state["stages"].as_array().unwrap();
    for name in ["collect", "plan", "implementation"] {
        let stage = stages.iter().find(|stage| stage["name"] == name).unwrap();
        let usage = &stage["usage"];
        assert!(
            usage["output"].as_u64().unwrap_or(0) > 0,
            "usage of {name}: {usage}"
        );
        assert!(usage["cost_usd"].is_number(), "cost of {name}: {usage}");
    }
    assert!(state["cost"]["total_usd"].as_f64().unwrap_or(0.0) > 0.0);
}

#[test]
fn ac9_every_row_has_a_session_the_session_manager_resolves() {
    let harness = dev_exemple_harness("native-ac9");
    let run_id = run_dev_exemple(&harness);
    let view = harness.view(&run_id);

    assert_eq!(view.session_ids().len(), DEV_EXEMPLE_LABELS.len());
    for row in view
        .stages
        .iter()
        .filter(|row| row.status == NativeStageStatus::Done)
    {
        assert!(
            !row.session_ids.is_empty(),
            "row {} has no session",
            row.name
        );
        for id in &row.session_ids {
            let record = harness
                .stack
                .sessions
                .get(id)
                .unwrap_or_else(|| panic!("session {id} of {} is unknown", row.name));
            assert_eq!(record.origin.kind, "run");
            assert_eq!(record.origin.r#ref.as_deref(), Some(run_id.as_str()));
            assert_eq!(record.origin.label.as_deref(), Some(row.name.as_str()));
        }
    }
}

/// Blocks the `ask` stage on its first launch with the question fixture, then lets it complete.
fn blocks_once(call: &Call) -> Behaviour {
    if call.label == ASK_STAGE && call.nth == 1 {
        Behaviour::raw(BLOCKED_REPLY)
    } else {
        Behaviour::ok()
    }
}

/// Blocks the `ask` stage on every launch.
fn blocks_always(call: &Call) -> Behaviour {
    if call.label == ASK_STAGE {
        Behaviour::raw(BLOCKED_REPLY)
    } else {
        Behaviour::ok()
    }
}

/// Replies to the `ask` stage with `reply` and lets every other stage complete.
fn replies_with(reply: &'static str) -> impl Fn(&Call) -> Behaviour + Send + Sync + 'static {
    move |call| {
        if call.label == ASK_STAGE {
            Behaviour::raw(reply)
        } else {
            Behaviour::ok()
        }
    }
}

/// The escalations a fixture reply declares, tagged with the stage that replied.
fn expected_escalations(reply: &str) -> Vec<Escalation> {
    let parsed: Value = serde_json::from_str(reply).unwrap();
    let entries = parsed["signals"]["escalations"].clone();
    let mut escalations: Vec<Escalation> = serde_json::from_value(entries).unwrap();
    for escalation in &mut escalations {
        escalation.stage = ASK_STAGE.to_string();
    }
    escalations
}

#[test]
fn autonomy_session_env_carries_the_switch_and_the_launch_disallows_questions() {
    let harness = plain_harness("native-auto-env", |_| Behaviour::ok());
    let run_id = harness.start_autonomous(&autonomy_template());
    assert_done(&harness.finish(&run_id));

    let specs = harness.spy.specs.lock().unwrap();
    assert_eq!(specs.len(), 2);
    for spec in specs.iter() {
        assert_eq!(
            spec.env.get(AUTONOMOUS_ENV).map(String::as_str),
            Some(AUTONOMOUS_ENABLED)
        );
        let flag = spec
            .extra_args
            .iter()
            .position(|argument| argument == DISALLOWED_TOOLS_FLAG)
            .expect("the launch carries the disallowed tools flag");
        assert_eq!(spec.extra_args[flag + 1], ASK_USER_QUESTION_TOOL);
    }
}

#[test]
fn autonomy_is_absent_from_a_run_that_is_not_autonomous() {
    let harness = plain_harness("native-auto-off", |_| Behaviour::ok());
    let run_id = harness.start(&autonomy_template(), &[]);
    assert_done(&harness.finish(&run_id));

    let specs = harness.spy.specs.lock().unwrap();
    assert_eq!(specs.len(), 2);
    for spec in specs.iter() {
        assert!(!spec.env.contains_key(AUTONOMOUS_ENV));
        assert!(!spec.extra_args.contains(&DISALLOWED_TOOLS_FLAG.to_string()));
    }
}

#[test]
fn autonomy_open_question_is_auto_answered_and_journaled_in_the_run_view() {
    let harness = plain_harness("native-auto-journal", blocks_once);
    let run_id = harness.start_autonomous(&autonomy_template());
    let view = harness.finish(&run_id);

    assert_done(&view);
    assert_eq!(harness.provider.count(ASK_STAGE), 2);
    assert_eq!(view.assumptions.len(), 1);
    let assumption = &view.assumptions[0];
    assert_eq!(assumption.stage, ASK_STAGE);
    assert_eq!(assumption.question_id.as_deref(), Some(QUESTION_ID));
    assert_eq!(assumption.question.as_deref(), Some(QUESTION_TEXT));
    assert_eq!(assumption.choice, RECOMMENDED_CHOICE);
    assert_eq!(assumption.reason, RECOMMENDED_REASON);
    assert!(assumption.auto_answered);
}

#[test]
fn autonomy_retry_prompt_lists_the_answer_with_its_question_and_choice() {
    let harness = plain_harness("native-auto-retry", blocks_once);
    let run_id = harness.start_autonomous(&autonomy_template());
    assert_done(&harness.finish(&run_id));

    let retry = harness
        .provider
        .calls()
        .into_iter()
        .filter(|call| call.label == ASK_STAGE)
        .nth(1)
        .expect("the stage is relaunched once");
    let expected = format!("- {QUESTION_ID} {QUESTION_TEXT} -> {RECOMMENDED_CHOICE}");
    assert!(retry.prompt.contains(RETRY_HEADING), "{}", retry.prompt);
    assert!(retry.prompt.contains(&expected), "{}", retry.prompt);
}

#[test]
fn autonomy_escalation_completes_the_stage_without_pausing_the_run() {
    let harness = plain_harness("native-auto-escalation", replies_with(ESCALATION_REPLY));
    let run_id = harness.start_autonomous(&autonomy_template());
    let view = harness.finish(&run_id);

    assert_done(&view);
    assert_eq!(harness.provider.count(ASK_STAGE), 1);
    assert_eq!(harness.provider.count(AFTER_STAGE), 1);
    assert_eq!(row_status(&view, ASK_STAGE), NativeStageStatus::Done);
    assert!(view.assumptions.is_empty());
    assert_eq!(view.escalations, expected_escalations(ESCALATION_REPLY));
}

#[test]
fn autonomy_stage_blocked_twice_fails_with_the_blocked_twice_reason() {
    let harness = plain_harness("native-auto-twice", blocks_always);
    let run_id = harness.start_autonomous(&autonomy_template());
    let view = harness.finish(&run_id);

    assert_eq!(view.status, NativeRunStatus::Paused, "{}", describe(&view));
    let row = view.stage(ASK_STAGE).unwrap();
    assert_eq!(row.status, NativeStageStatus::Failed);
    assert_eq!(row.reason.as_deref(), Some(BLOCKED_TWICE_REASON));
    assert_eq!(harness.provider.count(ASK_STAGE), 2);
    assert_eq!(harness.provider.count(AFTER_STAGE), 0);
}

#[test]
fn autonomy_three_thread_run_with_one_open_thread_completes_without_escalation() {
    let harness = plain_harness("native-auto-threads", replies_with(THREE_THREADS_REPLY));
    let run_id = harness.start_autonomous(&autonomy_template());
    let view = harness.finish(&run_id);

    assert_done(&view);
    assert_eq!(row_status(&view, ASK_STAGE), NativeStageStatus::Done);
    assert_eq!(row_status(&view, AFTER_STAGE), NativeStageStatus::Done);
    assert_eq!(harness.provider.count(ASK_STAGE), 1);
    assert_eq!(view.escalations, expected_escalations(THREE_THREADS_REPLY));
}

/// The permission mode each launched session asked for, in launch order.
fn asked_modes(harness: &Harness) -> Vec<String> {
    let specs = harness.spy.specs.lock().unwrap();
    specs
        .iter()
        .map(|spec| spec.extra_args.first().cloned().unwrap_or_default())
        .collect()
}

/// The directories one launch listed after `--add-dir`.
fn added_dirs(spec: &AgentSpec) -> Vec<String> {
    let position = spec
        .extra_args
        .iter()
        .position(|argument| argument == ADD_DIR_FLAG)
        .expect("the launch carries --add-dir");
    spec.extra_args[position + 1..]
        .iter()
        .take_while(|argument| !argument.starts_with("--"))
        .cloned()
        .collect()
}

#[test]
fn permissions_stage_sessions_ask_for_bypass_and_may_use_the_run_directory() {
    let harness = dev_exemple_harness("native-permissions-default");
    let run_id = run_dev_exemple(&harness);
    let run_dir = harness.view(&run_id).run_dir.to_string_lossy().to_string();

    let modes = asked_modes(&harness);
    assert_eq!(modes.len(), DEV_EXEMPLE_LABELS.len());
    assert!(modes.iter().all(|mode| mode == BYPASS_MODE), "{modes:?}");
    for spec in harness.spy.specs.lock().unwrap().iter() {
        assert_eq!(added_dirs(spec), vec![run_dir.clone()]);
    }
}

#[test]
fn permissions_a_refused_bypass_falls_back_once_for_the_whole_run() {
    let harness = dev_exemple_harness("native-permissions-fallback");
    harness.provider.refuse(Refusal {
        shown: DEFAULT_MODE.to_string(),
        bypass_only: true,
    });
    let run_id = run_dev_exemple(&harness);
    let view = harness.view(&run_id);

    let modes = asked_modes(&harness);
    assert_eq!(modes.len(), DEV_EXEMPLE_LABELS.len() + 1, "{modes:?}");
    assert_eq!(modes[0], BYPASS_MODE);
    assert!(
        modes[1..].iter().all(|mode| mode == AUTO_MODE),
        "bypass is asked once, then never again: {modes:?}"
    );
    let collect = view.stage("collect").expect("the collect row");
    assert_eq!(collect.session_ids.len(), 2);
    let refused = harness
        .stack
        .sessions
        .get(&collect.session_ids[0])
        .expect("the refused session is kept");
    assert_eq!(refused.status, SessionStatus::Cancelled);
}

#[test]
fn permissions_a_refused_fallback_names_both_modes_in_the_failure() {
    let harness = plain_harness("native-permissions-refused", |call| {
        if call.label == "draft" {
            Behaviour::raw("not json")
        } else {
            Behaviour::ok()
        }
    });
    harness.provider.refuse(Refusal {
        shown: DEFAULT_MODE.to_string(),
        bypass_only: false,
    });
    let run_id = harness.start(&fixture(GATED_TEMPLATE), &[ALL_GATES]);
    let view = harness.finish(&run_id);

    assert_eq!(view.status, NativeRunStatus::Paused, "{}", describe(&view));
    let reason = view
        .stage("draft")
        .unwrap()
        .reason
        .clone()
        .unwrap_or_default();
    assert!(reason.contains(REPLY_REJECTED), "{reason}");
    assert!(
        reason.contains(&format!(
            "`{DEFAULT_MODE}` permission mode instead of `{AUTO_MODE}`"
        )),
        "{reason}"
    );
}

#[test]
fn permissions_explicit_modes_are_used_on_every_stage_and_persisted() {
    for mode in PermissionMode::ALL {
        let harness = plain_harness(EXPLICIT_PERMISSIONS_TEST, |_| Behaviour::ok());
        let mut request = harness.request(&fixture(GATED_TEMPLATE), &[ALL_GATES]);
        request.permission_mode = Some(mode.as_str().to_string());
        let run_id = harness.stack.manager.start(request).unwrap();
        let view = harness.finish(&run_id);
        assert_done(&view);
        assert_eq!(view.request.permission_mode.as_deref(), Some(mode.as_str()));
        let modes = asked_modes(&harness);
        assert!(!modes.is_empty());
        assert!(
            modes.iter().all(|asked| asked == mode.as_str()),
            "{modes:?}"
        );
    }
}

#[test]
fn permissions_explicit_refusal_never_relaunches_with_fallback() {
    let harness = plain_harness(REFUSED_OVERRIDE_TEST, |_| Behaviour::ok());
    harness.provider.refuse(Refusal {
        shown: BYPASS_MODE.to_string(),
        bypass_only: false,
    });
    let mut request = harness.request(&fixture(GATED_TEMPLATE), &[ALL_GATES]);
    request.permission_mode = Some(PermissionMode::Default.as_str().to_string());
    let run_id = harness.stack.manager.start(request).unwrap();
    let view = harness.finish(&run_id);
    assert_eq!(view.status, NativeRunStatus::Failed);
    assert_eq!(asked_modes(&harness), vec![DEFAULT_MODE.to_string()]);
    assert_eq!(view.stage("draft").unwrap().session_ids.len(), 1);
    assert!(view
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains(aidp_gateway::models::constants::native_permissions::REFUSED_OVERRIDE));
}

#[test]
fn native_model_effort_and_policy_overrides_reach_every_stage() {
    let harness = plain_harness(MODEL_OVERRIDE_TEST, |_| Behaviour::ok());
    let mut request = harness.request(&fixture(GATED_TEMPLATE), &[ALL_GATES]);
    request.model = Some(MODEL_OVERRIDE.to_string());
    request.effort = Some(EFFORT_OVERRIDE.to_string());
    request.permission_mode = Some(PermissionMode::Default.as_str().to_string());
    request.strict_mcp_config = Some(true);
    request.mcp_servers = Some(Vec::new());
    let run_id = harness.stack.manager.start(request).unwrap();
    let view = harness.finish(&run_id);
    assert_done(&view);
    for spec in harness.spy.specs.lock().unwrap().iter() {
        assert_eq!(spec.model.as_deref(), Some(MODEL_OVERRIDE));
        assert_eq!(spec.effort.as_deref(), Some(EFFORT_OVERRIDE));
        assert_eq!(spec.strict_mcp_config, Some(true));
        assert_eq!(spec.mcp_servers, Some(Vec::new()));
        assert_eq!(
            spec.permission_policy,
            aidp_agents::PermissionPolicy::Default
        );
    }
}

/// A headless run asks `dontAsk` with the vault root added, through a restart and a retry.
#[test]
fn permissions_a_headless_run_never_prompts_through_a_restart_and_a_retry() {
    let draft_sessions = Arc::new(AtomicUsize::new(0));
    let counted = draft_sessions.clone();
    let harness = plain_harness(HEADLESS_TEST, move |call| {
        let failing = call.label == DRAFT_STAGE
            && counted.fetch_add(1, Ordering::SeqCst) < FAILED_DRAFT_SESSIONS;
        if failing {
            Behaviour::raw(MALFORMED_REPLY)
        } else {
            Behaviour::ok()
        }
    });
    harness.provider.refuse(Refusal {
        shown: DEFAULT_MODE.to_string(),
        bypass_only: true,
    });
    let request = StartNativeRun {
        headless: true,
        ..harness.request(&fixture(GATED_TEMPLATE), &[ALL_GATES])
    };
    let run_id = harness.stack.manager.start(request).unwrap();
    let paused = harness.finish(&run_id);
    assert_eq!(
        paused.status,
        NativeRunStatus::Paused,
        "{}",
        describe(&paused)
    );

    let restarted = harness.restarted_stack();
    assert!(restarted.manager.get(&run_id).unwrap().request.headless);
    restarted.manager.retry(&run_id, DRAFT_STAGE).unwrap();
    let view = restarted.manager.wait_for(&run_id, SETTLE).unwrap();
    assert_done(&view);

    let run_dir = view.run_dir.to_string_lossy().to_string();
    let vault_root = aidp_core::vault::resolve_global().expect("the harness sets a vault");
    let vault_root = vault_root.root.to_string_lossy().to_string();
    let modes = asked_modes(&harness);
    assert_eq!(
        modes.len(),
        FAILED_DRAFT_SESSIONS + SESSIONS_FROM_THE_RETRY,
        "{modes:?}"
    );
    assert!(modes.iter().all(|mode| mode == DONT_ASK_MODE), "{modes:?}");
    for spec in harness.spy.specs.lock().unwrap().iter() {
        assert_eq!(
            spec.permission_policy,
            aidp_agents::PermissionPolicy::Headless
        );
        assert_eq!(added_dirs(spec), vec![run_dir.clone(), vault_root.clone()]);
    }
    for session in view.session_ids() {
        assert!(restarted.sessions.get(&session).unwrap().headless);
    }
}

#[test]
fn vault_a_run_without_a_vault_keeps_its_directory_in_the_workspace() {
    let harness = dev_exemple_harness("native-no-vault");
    let request = StartNativeRun {
        no_vault: true,
        ..harness.request(&dev_exemple_template(), &[])
    };
    let run_id = harness
        .stack
        .manager
        .start(request)
        .expect("the run starts");
    let view = harness.finish(&run_id);

    assert_done(&view);
    assert!(
        view.run_dir
            .starts_with(harness.workspace.join(LEGACY_PIPELINES_DIR)),
        "{}",
        view.run_dir.display()
    );
}

#[test]
fn vault_a_requested_vault_wins_over_the_daemon_vault() {
    let harness = dev_exemple_harness("native-own-vault");
    let own = TempWorkspace::new("native-own-vault-root");
    let own_vault = own.child("vault");
    std::fs::create_dir_all(&own_vault).unwrap();
    let request = StartNativeRun {
        vault: Some(own_vault.clone()),
        ..harness.request(&dev_exemple_template(), &[])
    };
    let run_id = harness
        .stack
        .manager
        .start(request)
        .expect("the run starts");
    let view = harness.finish(&run_id);

    assert_done(&view);
    assert!(
        view.run_dir.starts_with(&own_vault),
        "{}",
        view.run_dir.display()
    );
}

#[test]
fn vault_a_run_without_a_choice_uses_the_daemon_vault() {
    let harness = dev_exemple_harness("native-daemon-vault");
    let run_id = run_dev_exemple(&harness);
    let daemon_vault = aidp_core::vault::resolve_global().expect("the harness sets a vault");

    assert!(harness
        .view(&run_id)
        .run_dir
        .starts_with(daemon_vault.pipelines_dir()));
}

fn step_outputs_template() -> PathBuf {
    fixture("step_outputs.yaml")
}

fn merged_outputs(view: &NativeRunView) -> BTreeMap<String, Value> {
    let mut outputs = BTreeMap::new();
    aidp_gateway::flows::step_outputs::merge_stage_outputs(&mut outputs, view);
    outputs
}

#[test]
fn step_outputs_expose_the_result_json_of_the_last_stage() {
    let harness = plain_harness("native-step-outputs", |call| {
        if call.label == "review" {
            std::fs::create_dir_all(&call.stage_dir).unwrap();
            std::fs::write(
                call.stage_dir.join("result.json"),
                r#"{"verdict":"done","passed":false,"pr":42,"branch":"feat/x","findings":[{"id":1}],"meta":{"k":"v"}}"#,
            )
            .unwrap();
        }
        Behaviour::ok()
    });
    let run_id = harness.start(&step_outputs_template(), &[]);
    let view = harness.finish(&run_id);
    assert_done(&view);
    let outputs = merged_outputs(&view);
    assert_eq!(outputs["verdict"], serde_json::json!("done"));
    assert_eq!(outputs["passed"], serde_json::json!(false));
    assert_eq!(outputs["pr"], serde_json::json!(42));
    assert_eq!(outputs["findings"], serde_json::json!([{"id": 1}]));
    assert_eq!(outputs["meta"], serde_json::json!({"k": "v"}));
}

#[test]
fn step_outputs_fall_back_to_the_signals_of_the_last_stage() {
    let harness = plain_harness("native-step-outputs-signals", |call| {
        if call.label == "review" {
            Behaviour::ok()
                .signal("verdict", serde_json::json!("need-fix"))
                .signal("passed", serde_json::json!(true))
        } else {
            Behaviour::ok()
        }
    });
    let run_id = harness.start(&step_outputs_template(), &[]);
    let view = harness.finish(&run_id);
    assert_done(&view);
    let outputs = merged_outputs(&view);
    assert_eq!(outputs["verdict"], serde_json::json!("need-fix"));
    assert_eq!(outputs["passed"], serde_json::json!(true));
}
