//! The pipeline doctor end to end: the real flow engine and native run executor, a scripted
//! provider for every stage, investigations included, and a fake `aidp` for the issue comment.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::fake_git::FakeGit;
use crate::native_runs::fake_provider::{Behaviour, Call, ScriptedProvider, PROVIDER_ID};
use aidp_agents::ProvidersConfig;
use aidp_core::clock::SystemClock;
use aidp_core::id::SystemIdGen;
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec, SpawnedChild};
use aidp_flows::{FlowDef, FlowState, FlowStatus};
use aidp_gateway::bus::Bus;
use aidp_gateway::doctor::ports::{DoctorRuns, HarnessRepos, NativeDoctorRuns, NoUnityModals};
use aidp_gateway::doctor::{Doctor, DoctorDeps};
use aidp_gateway::flows::executor::Ports;
use aidp_gateway::flows::lock::SessionLock;
use aidp_gateway::flows::registry::{AlwaysAllow, ManagerSessions, Registry};
use aidp_gateway::flows::{FlowEngine, FlowEngineConfig};
use aidp_gateway::launcher::RunLauncher;
use aidp_gateway::models::constants::doctor::{
    DIAGNOSIS_FILE, DOCTOR_TEMPLATE, EVENT_DOCTOR_INCIDENT,
};
use aidp_gateway::models::enums::native_run_status::NativeRunStatus;
use aidp_gateway::models::enums::{DoctorAction, DoctorIncidentStatus, DoctorTrigger};
use aidp_gateway::models::flow_execution_policy::FlowExecutionPolicy;
use aidp_gateway::models::native_run_start::StartNativeRun;
use aidp_gateway::models::native_run_view::NativeRunView;
use aidp_gateway::models::{DoctorCandidate, DoctorIncident, LocalLock, TicketFlowRequest};
use aidp_gateway::native_runs::{NativeDeps, NativeRunManager};
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::SessionManager;
use aidp_testkit::{EnvGuard, GitFixture, TempWorkspace};
use aidp_tickets::config::DoctorConfig;
use serde_json::{json, Value};
use tiny_http::Method;

const FLOW: &str = include_str!("fixtures/doctor/flow.yaml");
const SKILL_ROOT: &str = "tests/fixtures/doctor/skill";
pub(crate) const KEY: &str = "GH-1";
const IMPLEMENT: &str = "implement";
pub(crate) const DIAGNOSE: &str = "diagnose";
pub(crate) const MALFORMED_REPLY: &str = "not json";
pub(crate) const SESSIONS_OF_A_REJECTED_REPLY: usize = 2;
pub(crate) const SETTLE: Duration = Duration::from_secs(45);
pub(crate) const POLL: Duration = Duration::from_millis(20);
const CAUSE: &str = "the implement reply was rejected twice by a transient provider fault";
const MCP_TIMEOUT_REPLY: &str = "MCP server unity-mcp: connection timed out (CONNECT_TIMEOUT)";
const HARNESS_REPO: &str = "Litybe/aidp";
const HARNESS_FILE: &str = "crates/demo/lib.txt";
const MAIN_BRANCH: &str = "main";
const README_FILE: &str = "README.md";
const README_CONTENTS: &str = "game
";
const HARNESS_CONTENTS: &str = "one
";
const GAME_IDENTITY_NAME: &str = "game";
const GAME_IDENTITY_EMAIL: &str = "game@example.test";
const HARNESS_IDENTITY_NAME: &str = "h";
const HARNESS_IDENTITY_EMAIL: &str = "h@example.test";
const GAME_FIXTURE: GitFixture<'static> = GitFixture {
    files: &[(README_FILE, README_CONTENTS)],
    empty_directories: &[],
    branch: Some(MAIN_BRANCH),
    user_name: GAME_IDENTITY_NAME,
    user_email: GAME_IDENTITY_EMAIL,
    autocrlf_off: true,
};
const HARNESS_FIXTURE: GitFixture<'static> = GitFixture {
    files: &[(HARNESS_FILE, HARNESS_CONTENTS)],
    empty_directories: &[],
    branch: Some(MAIN_BRANCH),
    user_name: HARNESS_IDENTITY_NAME,
    user_email: HARNESS_IDENTITY_EMAIL,
    autocrlf_off: true,
};
pub(crate) const PR_URL: &str = "https://github.test/Litybe/aidp/pull/7";

/// The `aidp` verbs the doctor spawns: every call is recorded and answered `{}`.
#[derive(Default)]
pub(crate) struct FakeAidp {
    pub(crate) calls: Mutex<Vec<Vec<String>>>,
    pub(crate) comments: Mutex<Vec<String>>,
    /// Every `git` and `gh` command, program first.
    pub(crate) commands: Mutex<Vec<Vec<String>>>,
    /// Answers every `git` command from memory when set; real `git` otherwise.
    pub(crate) git: Option<Arc<FakeGit>>,
}

/// Which `git` the world runs on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GitMode {
    Fake,
    Real,
}

struct Finished {
    stdout: Vec<u8>,
}

impl SpawnedChild for Finished {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(Cursor::new(std::mem::take(&mut self.stdout))))
    }
    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        None
    }
    fn id(&self) -> Option<u32> {
        None
    }
    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(0)
    }
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(Some(0))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ProcessRunner for FakeAidp {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        let mut line = vec![spec.program.clone()];
        line.extend(spec.args.clone());
        match spec.program.as_str() {
            "git" => {
                self.commands.lock().unwrap().push(line);
                match &self.git {
                    Some(git) => git.output(spec),
                    None => OsProcessRunner.output(spec),
                }
            }
            "gh" => {
                self.commands.lock().unwrap().push(line);
                Ok(std::process::Output {
                    status: success(),
                    stdout: format!("{PR_URL}\n").into_bytes(),
                    stderr: Vec::new(),
                })
            }
            _ => Err(std::io::Error::other("unexpected output call")),
        }
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        let args = spec.args.clone();
        if let Some(index) = args.iter().position(|argument| argument == "--body-file") {
            let body = std::fs::read_to_string(&args[index + 1]).unwrap_or_default();
            self.comments.lock().unwrap().push(body);
        }
        self.calls.lock().unwrap().push(args);
        Ok(Box::new(Finished {
            stdout: b"{}".to_vec(),
        }))
    }
}

#[cfg(windows)]
fn success() -> std::process::ExitStatus {
    std::os::windows::process::ExitStatusExt::from_raw(0)
}

#[cfg(unix)]
fn success() -> std::process::ExitStatus {
    std::os::unix::process::ExitStatusExt::from_raw(0)
}

/// What the investigation answers.
pub(crate) enum Investigation {
    Diagnosis(Value),
    /// The diagnose stage replies garbage twice: the investigation run pauses.
    Broken,
}

pub(crate) struct World {
    pub(crate) engine: Arc<FlowEngine>,
    pub(crate) native: Arc<NativeRunManager>,
    pub(crate) provider: Arc<ScriptedProvider>,
    pub(crate) doctor: Arc<Doctor>,
    pub(crate) cli: Arc<FakeAidp>,
    pub(crate) bus: Arc<Bus>,
    pub(crate) project_id: String,
    pub(crate) harness_origin: PathBuf,
    pub(crate) workspace: PathBuf,
    pub(crate) origin: PathBuf,
    pub(crate) git: Option<Arc<FakeGit>>,
    #[allow(dead_code)]
    pub(crate) config: Arc<Mutex<DoctorConfig>>,
    #[allow(dead_code)]
    pub(crate) extra_runs: Arc<Mutex<Vec<NativeRunView>>>,
    #[allow(dead_code)]
    pub(crate) root: TempWorkspace,
    pub(crate) _env: EnvGuard,
}

pub(crate) fn run_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn behaviour(
    call: &Call,
    investigation: &Investigation,
    implement_failures: &AtomicUsize,
    failure_reply: &str,
) -> Behaviour {
    match call.label.as_str() {
        IMPLEMENT => {
            let failing = implement_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_ok();
            if failing {
                Behaviour::raw(failure_reply)
            } else {
                Behaviour::ok()
            }
        }
        DIAGNOSE => match investigation {
            Investigation::Diagnosis(diagnosis) => {
                std::fs::create_dir_all(&call.stage_dir).unwrap();
                std::fs::write(call.stage_dir.join(DIAGNOSIS_FILE), diagnosis.to_string()).unwrap();
                Behaviour::ok()
            }
            Investigation::Broken => Behaviour::raw(MALFORMED_REPLY),
        },
        _ => Behaviour::ok(),
    }
}

fn world(tag: &str, investigation: Investigation) -> World {
    world_failing(tag, investigation, SESSIONS_OF_A_REJECTED_REPLY)
}

fn world_failing(tag: &str, investigation: Investigation, failures: usize) -> World {
    world_full(tag, investigation, failures, MALFORMED_REPLY, &[])
}

/// A world on the in-memory `git` whose rejected replies say `failure_reply`, with `seed`
/// incidents already on disk.
pub(crate) fn world_full(
    tag: &str,
    investigation: Investigation,
    failures: usize,
    failure_reply: &'static str,
    seed: &[DoctorIncident],
) -> World {
    world_on(
        GitMode::Fake,
        tag,
        investigation,
        failures,
        failure_reply,
        seed,
    )
}

/// The same world on the real `git` binary.
pub(crate) fn world_on_real_git(tag: &str, investigation: Investigation, failures: usize) -> World {
    world_on(
        GitMode::Real,
        tag,
        investigation,
        failures,
        MALFORMED_REPLY,
        &[],
    )
}

fn world_on(
    mode: GitMode,
    tag: &str,
    investigation: Investigation,
    failures: usize,
    failure_reply: &'static str,
    seed: &[DoctorIncident],
) -> World {
    let root = TempWorkspace::new(tag);
    let vault = root.child("vault");
    let home = root.child("home");
    let workspace = root.child("workspace");
    let state_dir = root.child("state");
    for directory in [&vault, &home, &state_dir, &workspace] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let origin = root.child("origin.git");
    let fake = (mode == GitMode::Fake).then(|| Arc::new(FakeGit::new()));
    match &fake {
        Some(git) => git.seed(
            &workspace,
            Some(&origin),
            MAIN_BRANCH,
            &[(README_FILE, README_CONTENTS)],
        ),
        None => GAME_FIXTURE.copy_with_origin_into(&workspace, &origin),
    }
    let env = EnvGuard::with(&[
        ("AIDP_VAULT", Some(vault.to_str().unwrap())),
        ("HOME", Some(home.to_str().unwrap())),
        ("USERPROFILE", Some(home.to_str().unwrap())),
    ]);

    let projects = Arc::new(Projects::load(&state_dir));
    let project_id = projects.add(workspace.to_str().unwrap()).id;
    let bus = Arc::new(Bus::new());
    let implement_failures = AtomicUsize::new(failures);
    let provider = ScriptedProvider::new(None, move |call| {
        behaviour(call, &investigation, &implement_failures, failure_reply)
    });
    let session_runner: Arc<dyn ProcessRunner> = match &fake {
        Some(git) => git.clone(),
        None => Arc::new(OsProcessRunner),
    };
    let sessions =
        SessionManager::load_with_runner(&state_dir, bus.clone(), projects.clone(), session_runner)
            .with_provider(PROVIDER_ID, provider.clone())
            .with_providers_config(ProvidersConfig {
                default: Some(PROVIDER_ID.to_string()),
                ..ProvidersConfig::default()
            });
    let skill_root = Path::new(env!("CARGO_MANIFEST_DIR")).join(SKILL_ROOT);
    let mut deps = NativeDeps::new(
        sessions.clone(),
        projects.clone(),
        bus.clone(),
        Arc::new(AlwaysAllow),
        Arc::new(LocalLock::default()),
        &state_dir,
        Some(skill_root),
    );
    deps.poll_interval = Duration::from_millis(10);
    if let Some(git) = &fake {
        deps = deps.with_git(git.clone(), git.clone());
    }
    let native = NativeRunManager::new(deps);
    let launcher = Arc::new(RunLauncher::new(sessions.clone(), native.clone()));
    let cli = Arc::new(FakeAidp {
        git: fake.clone(),
        ..FakeAidp::default()
    });
    let ports = Ports {
        sessions: Arc::new(
            ManagerSessions::new(sessions.clone(), projects.clone(), launcher)
                .with_vault_root(vault.clone()),
        ),
        budget: Arc::new(AlwaysAllow),
        lock: Arc::new(SessionLock::new(sessions)),
        bus: bus.clone(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(SystemIdGen),
        runner: cli.clone(),
        lock_wait: Duration::from_secs(5),
    };
    let engine = FlowEngine::new(
        vault,
        state_dir.clone(),
        Registry::with_builtins(),
        ports,
        FlowEngineConfig::default(),
    );
    let harness_origin = root.child("harness-origin.git");
    let harness = root.child("harness");
    match &fake {
        Some(git) => git.seed(
            &harness,
            Some(&harness_origin),
            MAIN_BRANCH,
            &[(HARNESS_FILE, HARNESS_CONTENTS)],
        ),
        None => HARNESS_FIXTURE.copy_with_origin_into(&harness, &harness_origin),
    }
    let clone = harness.clone();
    let harness_repos: HarnessRepos =
        Arc::new(move |repo: &str| (repo == HARNESS_REPO).then(|| clone.clone()));
    if !seed.is_empty() {
        let directory = state_dir.join("doctor");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("incidents.json"),
            serde_json::to_string(seed).unwrap(),
        )
        .unwrap();
    }
    let config = Arc::new(Mutex::new(DoctorConfig::default()));
    let settings_config = config.clone();
    let extra_runs: Arc<Mutex<Vec<NativeRunView>>> = Arc::new(Mutex::new(Vec::new()));
    let doctor = Doctor::new(DoctorDeps {
        engine: engine.clone(),
        runs: Arc::new(ExtraRuns {
            inner: NativeDoctorRuns(native.clone()),
            extra: extra_runs.clone(),
        }),
        settings: Arc::new(move || settings_config.lock().unwrap().clone()),
        state_dir,
        harness_repos,
        unity: None,
        unity_modals: Arc::new(NoUnityModals),
        first_delay: Duration::ZERO,
        interval: Duration::from_secs(3600),
    });
    World {
        engine,
        native,
        provider,
        doctor,
        cli,
        bus,
        project_id,
        harness_origin,
        workspace,
        origin,
        git: fake,
        config,
        extra_runs,
        root,
        _env: env,
    }
}

/// The native runs of the daemon plus synthetic ones a test adds, to show the doctor other tickets' reports.
struct ExtraRuns {
    inner: NativeDoctorRuns,
    extra: Arc<Mutex<Vec<NativeRunView>>>,
}

impl DoctorRuns for ExtraRuns {
    fn list(&self) -> Vec<NativeRunView> {
        let mut runs = self.inner.list();
        runs.extend(self.extra.lock().unwrap().iter().cloned());
        runs
    }

    fn get(&self, run_id: &str) -> Option<NativeRunView> {
        self.extra
            .lock()
            .unwrap()
            .iter()
            .find(|view| view.run_id == run_id)
            .cloned()
            .or_else(|| self.inner.get(run_id))
    }

    fn start(&self, request: StartNativeRun) -> anyhow::Result<String> {
        self.inner.start(request)
    }

    fn resume(&self, run_id: &str) -> anyhow::Result<()> {
        self.inner.resume(run_id)
    }

    fn cancel_sessions(&self, run_id: &str) -> anyhow::Result<usize> {
        self.inner.cancel_sessions(run_id)
    }

    fn resume_from_stage(&self, run_id: &str, stage: &str) -> anyhow::Result<()> {
        self.inner.resume_from_stage(run_id, stage)
    }

    fn set_model(&self, run_id: &str, model: &str) -> anyhow::Result<()> {
        self.inner.set_model(run_id, model)
    }
}

fn diagnosis(action: &str, confidence: f64) -> Value {
    json!({
        "category": "other",
        "cause": CAUSE,
        "evidence": [{ "file": "evidence/native-run.json", "line": "reply rejected after one repair" }],
        "confidence": confidence,
        "action": action,
        "params": { "step": IMPLEMENT },
        "needs_human": false,
        "proposed_patch": null,
        "summary": "Échec transitoire du fournisseur ; relance de l'étape."
    })
}

fn diagnosis_of(category: &str, action: &str, confidence: f64, patch: Option<&str>) -> Value {
    let mut value = diagnosis(action, confidence);
    value["category"] = json!(category);
    value["proposed_patch"] = json!(patch);
    value["params"] = json!({ "repo": HARNESS_REPO });
    value
}

impl World {
    /// Runs `git args` in `directory` on the git of the world, failing the test on an error.
    pub(crate) fn run_git(&self, directory: &Path, arguments: &[&str]) {
        match &self.git {
            Some(git) => {
                git.run(directory, arguments).unwrap();
            }
            None => run_git(directory, arguments),
        }
    }

    /// The bytes of `revision:path` (or the commit text of `revision`) in the bare repository `origin`.
    pub(crate) fn show_in(&self, origin: &Path, spec: &str) -> Vec<u8> {
        match &self.git {
            Some(git) => git.show(origin, spec),
            None => {
                let output = Command::new("git")
                    .args(["--git-dir", origin.to_str().unwrap(), "show", spec])
                    .output()
                    .unwrap();
                assert!(output.status.success(), "git show {spec}");
                output.stdout
            }
        }
    }

    /// The branches of the bare repository `origin`, one per line.
    pub(crate) fn branches_of(&self, origin: &Path) -> String {
        match &self.git {
            Some(git) => git.run(origin, &["branch", "--list"]).unwrap(),
            None => branches(origin),
        }
    }

    pub(crate) fn start(&self) {
        self.start_def(KEY, FLOW);
    }

    /// Starts the flow of `yaml` for ticket `key`, in the project of the world.
    pub(crate) fn start_def(&self, key: &str, yaml: &str) {
        let ticket = json!({ "key": key, "service": self.project_id, "summary": "game feature" });
        let request = TicketFlowRequest {
            ticket,
            policy: Some(FlowExecutionPolicy {
                autonomous: true,
                provider: Some(PROVIDER_ID.to_string()),
                ..FlowExecutionPolicy::default()
            }),
            ..TicketFlowRequest::default()
        };
        self.engine
            .start_request_with_def(request, FlowDef::from_yaml(yaml).unwrap())
            .expect("the flow starts");
    }

    pub(crate) fn wait_flow(&self, status: FlowStatus) -> FlowState {
        self.wait_flow_of(KEY, status)
    }

    pub(crate) fn wait_flow_of(&self, key: &str, status: FlowStatus) -> FlowState {
        let deadline = Instant::now() + SETTLE;
        loop {
            self.engine.executor().reconcile_all();
            let state = self.engine.get(key).unwrap();
            if state.status == status {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "flow {:?}, wanted {status:?}; steps {:?}; calls {:?}",
                state.status,
                state.steps,
                self.provider.labels()
            );
            std::thread::sleep(POLL);
        }
    }

    /// The newest incident of the ticket.
    pub(crate) fn incident(&self) -> DoctorIncident {
        self.incident_of(KEY)
    }

    pub(crate) fn incident_of(&self, key: &str) -> DoctorIncident {
        let incidents = self.doctor.incidents_for(key);
        incidents.into_iter().next().expect("an incident")
    }

    /// Ticks the doctor until its incident leaves `Investigating`.
    pub(crate) fn investigate(&self) -> DoctorIncident {
        self.investigate_of(KEY)
    }

    pub(crate) fn investigate_of(&self, key: &str) -> DoctorIncident {
        self.doctor.tick();
        let started = self.incident_of(key);
        assert_eq!(
            started.status,
            DoctorIncidentStatus::Investigating,
            "{started:?}"
        );
        let run_id = started.investigation_run_id.clone().unwrap();
        let deadline = Instant::now() + SETTLE;
        loop {
            let view = self.native.wait_for(&run_id, POLL).unwrap();
            let settled = view.status.is_terminal() || view.status == NativeRunStatus::Paused;
            if settled {
                break;
            }
            assert!(Instant::now() < deadline, "investigation {view:?}");
        }
        self.doctor.tick();
        self.incident_of(key)
    }

    pub(crate) fn doctor_events(&self) -> Vec<Value> {
        self.bus
            .recent(500, 0)
            .into_iter()
            .filter(|event| event["type"] == EVENT_DOCTOR_INCIDENT)
            .collect()
    }
}

#[test]
fn a_failed_flow_is_diagnosed_then_retried_and_reported() {
    let world = world(
        "doctor-retry",
        Investigation::Diagnosis(diagnosis(DoctorAction::RetryStep.as_str(), 0.9)),
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);

    let closed = world.investigate();
    assert_eq!(closed.status, DoctorIncidentStatus::Fixed, "{closed:?}");
    assert_eq!(closed.candidate.trigger, DoctorTrigger::FlowFailed);
    assert_eq!(closed.candidate.step.as_deref(), Some(IMPLEMENT));
    let outcome = closed.outcome.clone().unwrap();
    assert_eq!(outcome.action, DoctorAction::RetryStep);
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(
        outcome.idempotency_key,
        format!("{}:action:1", closed.candidate.id)
    );
    assert_eq!(closed.actions.len(), 1);
    assert!(closed.commented);
    let bundle = PathBuf::from(closed.bundle.clone().unwrap());
    assert!(bundle.join("evidence/flow-state.json").is_file());
    assert!(bundle.join("evidence/native-run.json").is_file());

    world.wait_flow(FlowStatus::Done);
    assert_eq!(
        world.provider.count(IMPLEMENT),
        SESSIONS_OF_A_REJECTED_REPLY + 1
    );
    let comments = world.cli.comments.lock().unwrap().clone();
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert!(comments[0].contains(CAUSE), "{}", comments[0]);
    assert!(comments[0].contains("retry_step"), "{}", comments[0]);
    assert!(comments[0].contains("fixed"), "{}", comments[0]);
    let journal = world.engine.journal(KEY).unwrap();
    assert!(journal
        .iter()
        .any(|line| line.line.contains("retry implement by doctor")));
    assert!(world.doctor_events().len() >= 3);

    world.doctor.tick();
    assert_eq!(
        world.doctor.incidents().len(),
        1,
        "a done flow and the investigation open nothing"
    );
    let (status, body) = aidp_gateway::doctor::http_routes::handle(
        Some(&world.doctor),
        Some(&world.engine),
        &Method::Get,
        &format!("/api/flows/{KEY}/doctor"),
    )
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(body["incidents"][0]["status"], "fixed");
    assert_eq!(body["incidents"][0]["outcome"]["action"], "retry_step");
}

#[test]
fn an_action_outside_the_allowed_list_is_refused_and_nothing_runs() {
    let world = world(
        "doctor-refused",
        Investigation::Diagnosis(diagnosis("merge_pr", 1.0)),
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);

    let closed = world.investigate();
    assert_eq!(closed.status, DoctorIncidentStatus::Refused, "{closed:?}");
    assert!(closed.outcome.is_none());
    assert!(closed.note.unwrap().contains("merge_pr"));
    assert_eq!(world.engine.get(KEY).unwrap().status, FlowStatus::Failed);
    assert_eq!(
        world.provider.count(IMPLEMENT),
        SESSIONS_OF_A_REJECTED_REPLY,
        "{:?} {:?}",
        world.provider.labels(),
        world
            .engine
            .journal(KEY)
            .unwrap()
            .iter()
            .map(|line| line.line.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_low_confidence_diagnosis_asks_a_human_and_acts_on_nothing() {
    let world = world(
        "doctor-low-confidence",
        Investigation::Diagnosis(diagnosis(DoctorAction::RetryStep.as_str(), 0.3)),
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);

    let closed = world.investigate();
    assert_eq!(
        closed.status,
        DoctorIncidentStatus::NeedsHuman,
        "{closed:?}"
    );
    assert!(closed.outcome.is_none());
    assert_eq!(world.engine.get(KEY).unwrap().status, FlowStatus::Failed);
}

#[test]
fn a_failed_investigation_never_starts_another_doctor() {
    let world = world("doctor-no-recursion", Investigation::Broken);
    world.start();
    world.wait_flow(FlowStatus::Failed);

    let closed = world.investigate();
    assert_eq!(
        closed.status,
        DoctorIncidentStatus::InvestigationFailed,
        "{closed:?}"
    );
    let run_id = closed.investigation_run_id.clone().unwrap();
    assert_eq!(
        world.native.get(&run_id).unwrap().status,
        NativeRunStatus::Paused
    );
    assert_eq!(
        world.native.get(&run_id).unwrap().template_id,
        DOCTOR_TEMPLATE
    );

    for _ in 0..3 {
        world.doctor.tick();
    }
    let all = world.doctor.incidents();
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].status, DoctorIncidentStatus::InvestigationFailed);
    let investigations = world
        .native
        .list()
        .into_iter()
        .filter(|view| view.template_id == DOCTOR_TEMPLATE)
        .count();
    assert_eq!(investigations, 1);
}

#[test]
fn the_ticket_cap_stops_investigations_after_two_a_day() {
    let world = world_failing(
        "doctor-cap",
        Investigation::Diagnosis(diagnosis(DoctorAction::None.as_str(), 0.9)),
        3 * SESSIONS_OF_A_REJECTED_REPLY,
    );
    world.start();
    for round in 0..2 {
        world.wait_flow(FlowStatus::Failed);
        let closed = world.investigate();
        assert_eq!(
            closed.status,
            DoctorIncidentStatus::NoAction,
            "round {round}: {closed:?}"
        );
        world.engine.retry(KEY, IMPLEMENT, "test").unwrap();
    }
    world.wait_flow(FlowStatus::Failed);
    world.doctor.tick();
    let newest = world.doctor.incidents_for(KEY).into_iter().next().unwrap();
    assert_eq!(newest.status, DoctorIncidentStatus::Capped, "{newest:?}");
    assert!(newest.investigation_run_id.is_none());
    assert_eq!(world.doctor.incidents().len(), 3);
    let investigations = world
        .native
        .list()
        .into_iter()
        .filter(|view| view.template_id == DOCTOR_TEMPLATE)
        .count();
    assert_eq!(investigations, 2);
}

#[test]
fn a_failed_investigation_gets_one_more_after_the_delay_and_the_second_failure_is_final() {
    let mut first_failure = stale_incident(KEY);
    first_failure.candidate.id = format!("{KEY}/{IMPLEMENT}@0");
    first_failure.candidate.trigger = DoctorTrigger::FlowFailed;
    first_failure.candidate.step = Some(IMPLEMENT.to_string());
    first_failure.candidate.run_id = None;
    first_failure.status = DoctorIncidentStatus::InvestigationFailed;
    first_failure.investigations = 1;
    first_failure.investigation_failures = 1;
    first_failure.next_attempt_ms = Some(1);
    let world = world_full(
        "doctor-investigation-retry",
        Investigation::Broken,
        SESSIONS_OF_A_REJECTED_REPLY,
        MALFORMED_REPLY,
        &[first_failure],
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);

    let closed = world.investigate();
    assert_eq!(
        closed.status,
        DoctorIncidentStatus::InvestigationFailed,
        "{closed:?}"
    );
    assert_eq!(closed.investigation_failures, 2, "{closed:?}");
    assert_eq!(closed.next_attempt_ms, None);
    for _ in 0..3 {
        world.doctor.tick();
    }
    assert_eq!(doctor_runs(&world), 1);
    assert_eq!(
        world.incident().status,
        DoctorIncidentStatus::InvestigationFailed
    );
}

fn stale_incident(key: &str) -> DoctorIncident {
    DoctorIncident {
        candidate: DoctorCandidate {
            id: format!("{key}/produce@1"),
            key: key.to_string(),
            ticket: key.to_string(),
            trigger: DoctorTrigger::RunPaused,
            step: Some("produce".to_string()),
            iteration: Some(1),
            run_id: Some("20261108-old".to_string()),
            project_id: None,
            detail: "run 20261108-old (art-produce) paused: needs_review".to_string(),
            live: false,
            activity_ms: 0,
        },
        status: DoctorIncidentStatus::Detected,
        detected_at: "2026-11-08T10:00:00Z".to_string(),
        updated_at: "2026-11-08T10:00:00Z".to_string(),
        investigation_run_id: None,
        investigated_ms: None,
        bundle: None,
        diagnosis: None,
        outcome: None,
        actions: Vec::new(),
        next_attempt_ms: None,
        pr_url: None,
        note: None,
        commented: false,
        shared_pattern: None,
        affected: Vec::new(),
        rearmed_ms: None,
        investigations: 0,
        investigation_failures: 0,
    }
}

fn doctor_runs(world: &World) -> usize {
    world
        .native
        .list()
        .into_iter()
        .filter(|view| view.template_id == DOCTOR_TEMPLATE)
        .count()
}

#[test]
fn a_passing_fault_is_resumed_without_a_model_and_does_not_use_an_investigation() {
    let world = world_full(
        "doctor-transient",
        Investigation::Broken,
        SESSIONS_OF_A_REJECTED_REPLY,
        MCP_TIMEOUT_REPLY,
        &[],
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);

    world.doctor.tick();
    let incident = world.incident();
    assert_eq!(doctor_runs(&world), 0, "no investigation was started");
    assert!(incident.investigated_ms.is_none(), "{incident:?}");
    assert_eq!(incident.actions.len(), 1, "{incident:?}");
    let action = &incident.actions[0];
    assert_eq!(action.action, DoctorAction::RetryStep);
    assert!(action.ok, "{action:?}");
    assert_eq!(
        action.idempotency_key,
        format!("{}:action:1", incident.candidate.id)
    );
    assert_eq!(
        incident.diagnosis.as_ref().map(|d| d.category.as_str()),
        Some("mcp_connect_timeout")
    );

    world.wait_flow(FlowStatus::Done);
    assert_eq!(
        world.provider.count(IMPLEMENT),
        SESSIONS_OF_A_REJECTED_REPLY + 1,
        "the paused run was resumed, not restarted from scratch"
    );
    world.doctor.tick();
    let closed = world.incident();
    assert_eq!(closed.status, DoctorIncidentStatus::Fixed, "{closed:?}");
    assert!(closed.note.unwrap().contains("recovered"));
    assert_eq!(doctor_runs(&world), 0);
}

#[test]
fn obsolete_incidents_close_without_investigation_and_do_not_hold_the_live_one() {
    let world = world_full(
        "doctor-obsolete",
        Investigation::Diagnosis(diagnosis(DoctorAction::RetryStep.as_str(), 0.9)),
        SESSIONS_OF_A_REJECTED_REPLY,
        MALFORMED_REPLY,
        &[stale_incident("GH-90"), stale_incident("GH-91")],
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);

    world.doctor.tick();
    for key in ["GH-90", "GH-91"] {
        let stale = world.doctor.incidents_for(key).remove(0);
        assert_eq!(stale.status, DoctorIncidentStatus::NoAction, "{stale:?}");
        assert!(stale.note.unwrap().starts_with("obsolete"));
        assert!(stale.investigation_run_id.is_none());
    }
    let live = world.incident();
    assert_eq!(live.status, DoctorIncidentStatus::Investigating, "{live:?}");
    assert_eq!(
        doctor_runs(&world),
        1,
        "only the live failure is investigated"
    );
}

#[test]
fn a_product_bug_raises_a_bug_ticket_through_bug_create() {
    let world = world(
        "doctor-bug-ticket",
        Investigation::Diagnosis(diagnosis_of(
            "product_bug",
            DoctorAction::CreateBugTicket.as_str(),
            0.8,
            None,
        )),
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);

    let closed = world.investigate();
    assert_eq!(closed.status, DoctorIncidentStatus::Fixed, "{closed:?}");
    assert_eq!(closed.actions[0].action, DoctorAction::CreateBugTicket);
    let calls = world.cli.calls.lock().unwrap().clone();
    let bug = calls
        .iter()
        .find(|args| args.iter().any(|argument| argument == "bug-create"))
        .unwrap_or_else(|| panic!("no bug-create in {calls:?}"));
    assert!(bug.contains(&KEY.to_string()), "{bug:?}");
    assert_eq!(
        world.engine.get(KEY).unwrap().status,
        FlowStatus::Failed,
        "nothing else was done"
    );
}

#[test]
fn a_bug_ticket_is_refused_for_something_that_is_not_a_product_bug() {
    let world = world(
        "doctor-bug-ticket-refused",
        Investigation::Diagnosis(diagnosis_of(
            "other",
            DoctorAction::CreateBugTicket.as_str(),
            0.8,
            None,
        )),
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);
    let closed = world.investigate();
    assert_eq!(closed.status, DoctorIncidentStatus::Refused, "{closed:?}");
    assert!(world
        .cli
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|args| !args.iter().any(|argument| argument == "bug-create")));
}

const GOOD_PATCH: &str =
    "--- a/crates/demo/lib.txt\n+++ b/crates/demo/lib.txt\n@@ -1 +1 @@\n-one\n+two";
const BAD_PATCH: &str =
    "--- a/crates/demo/lib.txt\n+++ b/crates/demo/lib.txt\n@@ -1 +1 @@\n-absent\n+two";

fn branches(origin: &Path) -> String {
    let output = Command::new("git")
        .args(["branch", "--list"])
        .current_dir(origin)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn harness_pr_diagnosis() -> Investigation {
    Investigation::Diagnosis(diagnosis_of(
        "harness_defect",
        DoctorAction::OpenHarnessPr.as_str(),
        0.8,
        Some(GOOD_PATCH),
    ))
}

#[test]
fn a_harness_patch_becomes_a_pull_request_never_a_merge() {
    let world = world("doctor-harness-pr", harness_pr_diagnosis());
    a_harness_patch_opens_a_pull_request(&world);
    assert!(world.git.as_ref().unwrap().unsupported().is_empty());
}

#[test]
fn a_harness_patch_becomes_a_pull_request_never_a_merge_on_real_git() {
    let world = world_on_real_git(
        "doctor-harness-pr-real",
        harness_pr_diagnosis(),
        SESSIONS_OF_A_REJECTED_REPLY,
    );
    a_harness_patch_opens_a_pull_request(&world);
}

fn a_harness_patch_opens_a_pull_request(world: &World) {
    world.start();
    world.wait_flow(FlowStatus::Failed);

    let closed = world.investigate();
    assert_eq!(closed.status, DoctorIncidentStatus::Fixed, "{closed:?}");
    assert_eq!(closed.pr_url.as_deref(), Some(PR_URL));
    let remote = world.branches_of(&world.harness_origin);
    assert!(remote.contains("doctor/GH-1_implement_0"), "{remote}");
    assert!(!remote.contains("gamedev"), "{remote}");
    let commands = world.cli.commands.lock().unwrap().clone();
    let joined: Vec<String> = commands.iter().map(|line| line.join(" ")).collect();
    assert!(
        joined
            .iter()
            .any(|line| line.contains("worktree add") && line.contains("origin/main")),
        "{joined:?}"
    );
    assert!(joined.iter().any(|line| line.starts_with("gh pr create")));
    assert!(
        joined
            .iter()
            .filter(|line| line.contains(" push "))
            .all(|line| {
                !line.contains("--force") && !line.contains(" -f ") && !line.contains(" +")
            }),
        "{joined:?}"
    );
    assert!(
        joined
            .iter()
            .all(|line| !line.starts_with("gh pr merge") && !line.starts_with("git merge ")),
        "{joined:?}"
    );
    let main = world.show_in(
        &world.harness_origin,
        &format!("{MAIN_BRANCH}:{HARNESS_FILE}"),
    );
    assert_eq!(
        String::from_utf8_lossy(&main).trim(),
        "one",
        "main is untouched"
    );
}

#[test]
fn a_harness_patch_that_does_not_apply_is_noted_then_a_human() {
    let world = world(
        "doctor-harness-bad-patch",
        Investigation::Diagnosis(diagnosis_of(
            "harness_defect",
            DoctorAction::OpenHarnessPr.as_str(),
            0.8,
            Some(BAD_PATCH),
        )),
    );
    world.start();
    world.wait_flow(FlowStatus::Failed);

    let closed = world.investigate();
    assert_eq!(
        closed.status,
        DoctorIncidentStatus::NeedsHuman,
        "{closed:?}"
    );
    assert!(closed.note.unwrap().contains("does not apply"));
    assert!(closed.pr_url.is_none());
    assert!(!world.branches_of(&world.harness_origin).contains("doctor/"));
    let commands = world.cli.commands.lock().unwrap().clone();
    assert!(
        commands.iter().all(|line| line[0] != "gh"),
        "no PR was opened"
    );
    assert!(commands
        .iter()
        .all(|line| !(line[0] == "git" && line.contains(&"push".to_string()))));
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(git) = &self.git {
            let unsupported = git.unsupported();
            if !std::thread::panicking() {
                assert!(unsupported.is_empty(), "fake git: {unsupported:?}");
            }
        }
    }
}
