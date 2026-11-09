//! The pipeline doctor end to end: the real flow engine and native run executor, a scripted
//! provider for every stage, investigations included, and a fake `aidp` for the issue comment.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::native_runs::fake_provider::{Behaviour, Call, ScriptedProvider, PROVIDER_ID};
use aidp_agents::ProvidersConfig;
use aidp_core::clock::SystemClock;
use aidp_core::id::SystemIdGen;
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec, SpawnedChild};
use aidp_flows::{FlowDef, FlowState, FlowStatus};
use aidp_gateway::bus::Bus;
use aidp_gateway::doctor::ports::NativeDoctorRuns;
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
use aidp_gateway::models::{DoctorIncident, LocalLock, TicketFlowRequest};
use aidp_gateway::native_runs::{NativeDeps, NativeRunManager};
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::SessionManager;
use aidp_testkit::{EnvGuard, TempWorkspace};
use aidp_tickets::config::DoctorConfig;
use serde_json::{json, Value};
use tiny_http::Method;

const FLOW: &str = include_str!("fixtures/doctor/flow.yaml");
const SKILL_ROOT: &str = "tests/fixtures/doctor/skill";
const KEY: &str = "GH-1";
const IMPLEMENT: &str = "implement";
const DIAGNOSE: &str = "diagnose";
const MALFORMED_REPLY: &str = "not json";
const SESSIONS_OF_A_REJECTED_REPLY: usize = 2;
const SETTLE: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(20);
const CAUSE: &str = "the implement reply was rejected twice by a transient provider fault";

/// The `aidp` verbs the doctor spawns: every call is recorded and answered `{}`.
#[derive(Default)]
struct FakeAidp {
    calls: Mutex<Vec<Vec<String>>>,
    comments: Mutex<Vec<String>>,
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
    fn output(&self, _: &ProcessSpec) -> std::io::Result<std::process::Output> {
        Err(std::io::Error::other("unexpected output call"))
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

/// What the investigation answers.
enum Investigation {
    Diagnosis(Value),
    /// The diagnose stage replies garbage twice: the investigation run pauses.
    Broken,
}

struct World {
    engine: Arc<FlowEngine>,
    native: Arc<NativeRunManager>,
    provider: Arc<ScriptedProvider>,
    doctor: Arc<Doctor>,
    cli: Arc<FakeAidp>,
    bus: Arc<Bus>,
    project_id: String,
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
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn behaviour(
    call: &Call,
    investigation: &Investigation,
    implement_failures: &AtomicUsize,
) -> Behaviour {
    match call.label.as_str() {
        IMPLEMENT => {
            let failing = implement_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_ok();
            if failing {
                Behaviour::raw(MALFORMED_REPLY)
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
    let root = TempWorkspace::new(tag);
    let vault = root.child("vault");
    let home = root.child("home");
    let workspace = root.child("workspace");
    let state_dir = root.child("state");
    for directory in [&vault, &home, &state_dir, &workspace] {
        std::fs::create_dir_all(directory).unwrap();
    }
    std::fs::write(workspace.join("README.md"), "game\n").unwrap();
    run_git(&workspace, &["init", "-q", "-b", "main"]);
    run_git(&workspace, &["config", "user.email", "game@example.test"]);
    run_git(&workspace, &["config", "user.name", "game"]);
    run_git(&workspace, &["add", "."]);
    run_git(&workspace, &["commit", "-q", "-m", "initial"]);
    let origin = root.child("origin.git");
    std::fs::create_dir_all(&origin).unwrap();
    run_git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    run_git(
        &workspace,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    run_git(&workspace, &["push", "-q", "-u", "origin", "main"]);
    run_git(&workspace, &["remote", "set-head", "origin", "main"]);
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
        behaviour(call, &investigation, &implement_failures)
    });
    let sessions = SessionManager::load_with_runner(
        &state_dir,
        bus.clone(),
        projects.clone(),
        Arc::new(OsProcessRunner),
    )
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
    let native = NativeRunManager::new(deps);
    let launcher = Arc::new(RunLauncher::new(sessions.clone(), native.clone()));
    let cli = Arc::new(FakeAidp::default());
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
    let doctor = Doctor::new(DoctorDeps {
        engine: engine.clone(),
        runs: Arc::new(NativeDoctorRuns(native.clone())),
        settings: Arc::new(DoctorConfig::default),
        state_dir,
        unity: None,
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
        _root: root,
        _env: env,
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

impl World {
    fn start(&self) {
        let ticket = json!({ "key": KEY, "service": self.project_id, "summary": "game feature" });
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
            .start_request_with_def(request, FlowDef::from_yaml(FLOW).unwrap())
            .expect("the flow starts");
    }

    fn wait_flow(&self, status: FlowStatus) -> FlowState {
        let deadline = Instant::now() + SETTLE;
        loop {
            self.engine.executor().reconcile_all();
            let state = self.engine.get(KEY).unwrap();
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
    fn incident(&self) -> DoctorIncident {
        let incidents = self.doctor.incidents_for(KEY);
        incidents.into_iter().next().expect("an incident")
    }

    /// Ticks the doctor until its incident leaves `Investigating`.
    fn investigate(&self) -> DoctorIncident {
        self.doctor.tick();
        let started = self.incident();
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
        self.incident()
    }

    fn doctor_events(&self) -> Vec<Value> {
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
        format!("{}:action", closed.candidate.id)
    );
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
