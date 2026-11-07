//! A launched session must reach the `aidp` binary: `start` hands the child
//! an `AIDP_BIN` and a `PATH` that resolve it, and refuses to launch when the
//! binary cannot be found.

use std::collections::BTreeMap;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aidp_core::process_runner::{ProcessRunner, ProcessSpec, ScriptedProcessRunner, SpawnedChild};
use aidp_gateway::aidp_bin::{AIDP_BIN_ENV, PATH_ENV};
use aidp_gateway::bus::Bus;
use aidp_gateway::http::{serve_on, HttpConfig};
use aidp_gateway::models::enums::SessionStatus;
use aidp_gateway::projects::Projects;
use aidp_gateway::session_error::SessionError;
use aidp_gateway::sessions::{Launch, SessionManager, StartRequest};
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/claude_session.jsonl");
const CLAUDE_BIN_ENV: &str = "AIDP_CLAUDE_BIN";
const FAKE_BINARY_NAME: &str = "aidp-fake-binary";
const MISSING_BINARY_NAME: &str = "missing-aidp-binary";
const CONFIG_ERROR_CODE: &str = "config";
const SERVICE_UNAVAILABLE: u16 = 503;
const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";
const AMBIENT_SKILL: &str = "ambient/ai-dev-pipeline-v6";
const PINNED_TEMPLATE: &str = "dev-aidp";
const EXPECTED_COMMAND: &str = "/run-pipeline-v7 dev-aidp";

/// Records the environment of every spawned child, then replays a scripted
/// session.
struct RecordingRunner {
    inner: ScriptedProcessRunner,
    recorded_envs: Mutex<Vec<BTreeMap<String, String>>>,
    spawn_count: AtomicUsize,
}

impl RecordingRunner {
    fn new() -> Self {
        Self {
            inner: ScriptedProcessRunner::new().with_spawn_stdout(FIXTURE.as_bytes().to_vec()),
            recorded_envs: Mutex::new(Vec::new()),
            spawn_count: AtomicUsize::new(0),
        }
    }

    fn spawn_count(&self) -> usize {
        self.spawn_count.load(Ordering::SeqCst)
    }

    fn last_envs(&self) -> BTreeMap<String, String> {
        self.recorded_envs
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("no child was spawned")
    }
}

impl ProcessRunner for RecordingRunner {
    fn output(&self, spec: &ProcessSpec) -> io::Result<std::process::Output> {
        self.inner.output(spec)
    }

    fn spawn(&self, spec: &ProcessSpec) -> io::Result<Box<dyn SpawnedChild>> {
        self.spawn_count.fetch_add(1, Ordering::SeqCst);
        self.recorded_envs.lock().unwrap().push(spec.envs.clone());
        self.inner.spawn(spec)
    }
}

struct Fixture {
    _workspace: TempWorkspace,
    manager: Arc<SessionManager>,
    runner: Arc<RecordingRunner>,
    project_id: String,
    empty_path_dir: PathBuf,
    binary_dir: PathBuf,
}

fn start_request(project_id: String) -> StartRequest {
    StartRequest {
        project_id,
        shared_writer_group: None,
        allowed_tools: Vec::new(),
        add_dirs: Vec::new(),
        engine: Some("prompt".to_string()),
        template_id: None,
        target: None,
        inputs: None,
        prompt: Some("hello from the aidp binary test".to_string()),
        permission_mode: None,
        model: None,
        effort: None,
        max_budget_usd: None,
        origin: None,
        automation_id: None,
        automation_run_id: None,
        resume_from: None,
        auto_close: None,
        priority: None,
        context_from: None,
        auto_approve_gates: None,
        inputs_file: None,
        worktree: None,
        workspace: None,
        title: None,
        external: None,
        env: Default::default(),
        provider: None,
        ..StartRequest::default()
    }
}

fn fixture(name: &str) -> Fixture {
    let workspace = TempWorkspace::new(name);
    let state_dir = workspace.child("state");
    let project_dir = workspace.child("project");
    let empty_path_dir = workspace.child("empty-path");
    let binary_dir = workspace.child("binary");
    for dir in [&state_dir, &project_dir, &empty_path_dir, &binary_dir] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let runner = Arc::new(RecordingRunner::new());
    let shared_runner: Arc<dyn ProcessRunner> = runner.clone();
    let manager = SessionManager::load_with_runner(&state_dir, bus, projects, shared_runner);
    Fixture {
        _workspace: workspace,
        manager,
        runner,
        project_id: project.id.clone(),
        empty_path_dir,
        binary_dir,
    }
}

fn write_fake_binary(dir: &Path) -> PathBuf {
    let binary = dir.join(FAKE_BINARY_NAME);
    std::fs::write(&binary, b"").unwrap();
    binary
}

fn wait_until_not_running(manager: &Arc<SessionManager>, session_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(record) = manager.get(session_id) {
            if record.status != SessionStatus::Running {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "session never left the running state"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start_and_settle(fixture: &Fixture) {
    let launch = fixture
        .manager
        .start(start_request(fixture.project_id.clone()))
        .expect("start must succeed");
    match launch {
        Launch::Started(record) => wait_until_not_running(&fixture.manager, &record.id),
        Launch::Queued(_) => panic!("expected an immediate launch, got a queued one"),
    }
}

fn first_path_entry(envs: &BTreeMap<String, String>) -> PathBuf {
    let path_value = envs.get(PATH_ENV).expect("PATH missing from child env");
    std::env::split_paths(path_value)
        .next()
        .expect("PATH has no entry")
}

#[test]
fn start_gives_the_child_an_aidp_bin_and_path_that_resolve_the_binary_when_path_lacks_aidp() {
    let fixture = fixture("aidp_bin_current_exe");
    let empty_path = fixture.empty_path_dir.to_str().unwrap().to_string();
    let _guard = EnvGuard::with(&[(PATH_ENV, Some(&empty_path)), (AIDP_BIN_ENV, None)]);

    start_and_settle(&fixture);

    let envs = fixture.runner.last_envs();
    let binary = PathBuf::from(envs.get(AIDP_BIN_ENV).expect("AIDP_BIN missing"));
    assert!(binary.is_file(), "AIDP_BIN must name an existing file");
    assert_eq!(first_path_entry(&envs), binary.parent().unwrap());
}

#[test]
fn start_respects_a_user_defined_aidp_bin() {
    let fixture = fixture("aidp_bin_user_defined");
    let binary = write_fake_binary(&fixture.binary_dir);
    let empty_path = fixture.empty_path_dir.to_str().unwrap().to_string();
    let binary_value = binary.to_str().unwrap().to_string();
    let _guard = EnvGuard::with(&[
        (PATH_ENV, Some(&empty_path)),
        (AIDP_BIN_ENV, Some(&binary_value)),
    ]);

    start_and_settle(&fixture);

    let envs = fixture.runner.last_envs();
    assert_eq!(envs.get(AIDP_BIN_ENV), Some(&binary_value));
    assert_eq!(first_path_entry(&envs), fixture.binary_dir);
}

#[test]
fn embedded_session_uses_its_pinned_skill_and_helper_over_ambient_configuration() {
    let mut fixture = fixture("aidp_embedded_skill");
    let binary = write_fake_binary(&fixture.binary_dir);
    let binary_value = binary.to_string_lossy().to_string();
    let skill_value = aidp_testkit::fixtures::skill_root()
        .to_string_lossy()
        .to_string();
    fixture.manager = fixture
        .manager
        .clone()
        .with_runtime_environment(BTreeMap::from([
            (AIDP_BIN_ENV.to_string(), binary_value.clone()),
            (SKILL_ROOT_ENV.to_string(), skill_value.clone()),
            (
                PATH_ENV.to_string(),
                fixture.empty_path_dir.to_string_lossy().to_string(),
            ),
        ]));
    let _guard = EnvGuard::with(&[
        (SKILL_ROOT_ENV, Some(AMBIENT_SKILL)),
        (AIDP_BIN_ENV, Some(MISSING_BINARY_NAME)),
    ]);
    let mut request = start_request(fixture.project_id.clone());
    request.engine = Some(aidp_gateway::models::constants::engine::ENGINE_WORKFLOW.to_string());
    request.template_id = Some(PINNED_TEMPLATE.to_string());
    match fixture.manager.start(request).unwrap() {
        Launch::Started(record) => {
            assert_eq!(record.prompt.lines().next(), Some(EXPECTED_COMMAND));
            wait_until_not_running(&fixture.manager, &record.id);
        }
        Launch::Queued(_) => panic!("expected an immediate launch"),
    }
    let environment = fixture.runner.last_envs();
    assert_eq!(environment.get(AIDP_BIN_ENV), Some(&binary_value));
    assert_eq!(environment.get(SKILL_ROOT_ENV), Some(&skill_value));
    assert_eq!(std::env::var(SKILL_ROOT_ENV).unwrap(), AMBIENT_SKILL);
}

#[test]
fn start_refuses_the_launch_when_the_user_defined_binary_does_not_exist() {
    let fixture = fixture("aidp_bin_missing");
    let missing = fixture.binary_dir.join(MISSING_BINARY_NAME);
    let missing_value = missing.to_str().unwrap().to_string();
    let _guard = EnvGuard::with(&[(AIDP_BIN_ENV, Some(&missing_value))]);

    let error = match fixture
        .manager
        .start(start_request(fixture.project_id.clone()))
    {
        Ok(_) => panic!("a missing binary must refuse the launch"),
        Err(error) => error,
    };

    let session_error = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SessionError>())
        .expect("the error must downcast to SessionError");
    assert!(matches!(session_error, SessionError::Config(_)));
    assert!(
        session_error.to_string().contains(&missing_value),
        "message must name the path: {session_error}"
    );
    assert_eq!(fixture.runner.spawn_count(), 0);
    assert_eq!(fixture.manager.queue_depth(), 0);
}

#[test]
fn post_sessions_answers_503_config_naming_the_missing_aidp_binary() {
    let workspace = TempWorkspace::new("aidp_bin_http");
    let state_dir = workspace.path().to_path_buf();
    std::fs::write(state_dir.join(".defaults-seeded"), "").unwrap();
    let project_dir = state_dir.join("repo");
    std::fs::create_dir_all(&project_dir).unwrap();
    let missing_aidp = state_dir.join(MISSING_BINARY_NAME);
    let missing_claude = state_dir.join("missing-claude-binary");
    let missing_aidp_value = missing_aidp.to_str().unwrap().to_string();
    let missing_claude_value = missing_claude.to_str().unwrap().to_string();
    let _guard = EnvGuard::with(&[
        (AIDP_BIN_ENV, Some(&missing_aidp_value)),
        (CLAUDE_BIN_ENV, Some(&missing_claude_value)),
    ]);

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = serve_on(
        listener,
        HttpConfig {
            state_dir: state_dir.clone(),
            vault_root: PathBuf::new(),
            hook_token: "hook-secret".to_string(),
            ui_token: "ui-secret".to_string(),
            read_only: false,
            allowed_origins: vec![],
        },
    )
    .expect("serve_on failed");

    let project = post(
        port,
        "/api/projects",
        json!({ "path": project_dir.to_string_lossy() }),
    );
    assert_eq!(project.0, 200, "project registration failed: {}", project.1);
    let project_id = project.1["id"].as_str().unwrap().to_string();

    let (status, body) = post(
        port,
        "/api/sessions",
        json!({ "projectId": project_id, "engine": "prompt", "prompt": "x" }),
    );

    assert_eq!(status, SERVICE_UNAVAILABLE, "unexpected body: {body}");
    assert_eq!(body["code"], CONFIG_ERROR_CODE);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains(&missing_aidp_value),
        "error must name the path: {body}"
    );
    handle.stop();
}

fn post(port: u16, path: &str, body: Value) -> (u16, Value) {
    let response = ureq::post(&format!("http://127.0.0.1:{port}{path}"))
        .send_json(body)
        .unwrap_or_else(|error| error.into_response().expect("transport error"));
    let status = response.status();
    (status, response.into_json().expect("invalid JSON response"))
}
