use aidp_core::clock::SystemClock;
use aidp_core::id::SystemIdGen;
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec};
use aidp_flows::{ActionOutcome, FlowDef, FlowStatus};
use aidp_gateway::bus::Bus;
use aidp_gateway::flows::action_processes::FlowActionProcesses;
use aidp_gateway::flows::executor::Ports;
use aidp_gateway::flows::lock::LocalLock;
use aidp_gateway::flows::recovery::resume_all;
use aidp_gateway::flows::registry::{
    ActionContext, AlwaysAllow, FlowAction, NoopAction, PipelineLaunch, ProjectRef, Registry,
    SessionPort, SessionStart, SessionState,
};
use aidp_gateway::flows::{FlowEngine, FlowEngineConfig};
use aidp_testkit::TempWorkspace;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ACTION: &str = "owned-process";
const KEY: &str = "TEST-41";
const REPO: &str = "shutdown-repo";
const VAULT: &str = "vault";
const STATE: &str = "state";
const ROOT_PID: &str = "parent.pid";
const CHILD_PID: &str = "child.pid";
const FLOW: &str = include_str!("fixtures/flow_shutdown/action.yaml");
const LIMIT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(25);
const FIXTURES: &str = "tests/fixtures/flow_shutdown";
const EXIT_PARENT: &str = "exit";
#[cfg(unix)]
const STAY_PARENT: &str = "stay";
const NO_PIPELINE: &str = "unexpected pipeline";

struct Sessions(PathBuf);
impl SessionPort for Sessions {
    fn project_for(&self, _: &Value) -> Option<ProjectRef> {
        Some(ProjectRef {
            id: REPO.into(),
            repo: REPO.into(),
            path: self.0.clone(),
        })
    }
    fn start(&self, _: &PipelineLaunch) -> anyhow::Result<SessionStart> {
        anyhow::bail!(NO_PIPELINE)
    }
    fn state(&self, _: &str) -> SessionState {
        SessionState::Gone
    }
    fn cancel(&self, _: &str) {}
}

#[derive(Serialize)]
struct Ticket {
    key: &'static str,
}

struct ChildAction(ProcessSpec);
impl FlowAction for ChildAction {
    fn writes(&self) -> bool {
        true
    }
    fn run(&self, context: &ActionContext<'_>, _: &BTreeMap<String, Value>) -> ActionOutcome {
        let result = context
            .runner
            .spawn(&self.0)
            .and_then(|mut child| child.wait());
        match result {
            Ok(_) => ActionOutcome::Ok(BTreeMap::new()),
            Err(error) => ActionOutcome::TechnicalError(error.to_string()),
        }
    }
}

fn engine(workspace: &TempWorkspace, action: Arc<dyn FlowAction>) -> Arc<FlowEngine> {
    let mut registry = Registry::with_builtins();
    registry.register_action(ACTION, action);
    FlowEngine::new(
        workspace.child(VAULT),
        workspace.child(STATE),
        registry,
        Ports {
            sessions: Arc::new(Sessions(workspace.path().to_path_buf())),
            budget: Arc::new(AlwaysAllow),
            lock: Arc::new(LocalLock::default()),
            bus: Arc::new(Bus::new()),
            clock: Arc::new(SystemClock),
            ids: Arc::new(SystemIdGen),
            runner: Arc::new(OsProcessRunner),
            lock_wait: LIMIT,
        },
        FlowEngineConfig::default(),
    )
}

fn process_spec(workspace: &TempWorkspace, exit_parent: bool) -> ProcessSpec {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURES);
    #[cfg(windows)]
    let mut spec = {
        let mut arguments = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
            fixtures.join("parent.ps1").display().to_string(),
            workspace.child(ROOT_PID).display().to_string(),
            workspace.child(CHILD_PID).display().to_string(),
            fixtures.join("child.ps1").display().to_string(),
        ];
        if exit_parent {
            arguments.push("-ExitParent".into());
        }
        ProcessSpec::new("powershell").args(arguments)
    };
    #[cfg(unix)]
    let mut spec = ProcessSpec::new("sh").args([
        fixtures.join("parent.sh").display().to_string(),
        workspace.child(ROOT_PID).display().to_string(),
        workspace.child(CHILD_PID).display().to_string(),
        fixtures.join("child.sh").display().to_string(),
        if exit_parent {
            EXIT_PARENT
        } else {
            STAY_PARENT
        }
        .into(),
    ]);
    spec.owned_process_group = true;
    spec
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + LIMIT;
    while !condition() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(POLL);
    }
}

fn pid(workspace: &TempWorkspace, name: &str) -> u32 {
    wait_for(|| {
        std::fs::read_to_string(workspace.child(name))
            .is_ok_and(|value| value.parse::<u32>().is_ok())
    });
    std::fs::read_to_string(workspace.child(name))
        .unwrap()
        .parse()
        .unwrap()
}

fn alive(process_id: u32) -> bool {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {process_id}"), "/NH"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .any(|word| word == process_id.to_string())
    }
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &process_id.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
}

#[test]
fn shutdown_kills_owned_child_and_descendants_and_preserves_resumable_flow() {
    let workspace = TempWorkspace::new(ACTION);
    let engine = engine(
        &workspace,
        Arc::new(ChildAction(process_spec(&workspace, false))),
    );
    engine
        .start_with_def(
            serde_json::to_value(Ticket { key: KEY }).unwrap(),
            FlowDef::from_yaml(FLOW).unwrap(),
            None,
        )
        .unwrap();
    let parent = pid(&workspace, ROOT_PID);
    let descendant = pid(&workspace, CHILD_PID);
    assert!(alive(parent));
    assert!(alive(descendant));
    let before = serde_json::to_value(engine.get(KEY).unwrap()).unwrap();
    assert_eq!(engine.get(KEY).unwrap().status, FlowStatus::Running);
    engine.shutdown().unwrap();
    wait_for(|| !alive(parent) && !alive(descendant));
    assert!(engine.executor().is_idle());
    assert_eq!(
        serde_json::to_value(engine.get(KEY).unwrap()).unwrap(),
        before
    );
    assert!(engine
        .start_with_def(
            serde_json::to_value(Ticket { key: KEY }).unwrap(),
            FlowDef::from_yaml(FLOW).unwrap(),
            None
        )
        .is_err());
    let restarted = self::engine(&workspace, Arc::new(NoopAction));
    assert_eq!(resume_all(&restarted), 1);
    wait_for(|| restarted.get(KEY).unwrap().status == FlowStatus::Done);
    restarted.shutdown().unwrap();
}

#[test]
fn owned_tree_shutdown_handles_parent_already_exited_and_repeat_shutdown() {
    let workspace = TempWorkspace::new(EXIT_PARENT);
    let runner = FlowActionProcesses::new(Arc::new(OsProcessRunner));
    let mut parent = runner.spawn(&process_spec(&workspace, true)).unwrap();
    let descendant = pid(&workspace, CHILD_PID);
    wait_for(|| parent.try_wait().unwrap().is_some());
    assert!(alive(descendant));
    runner.shutdown().unwrap();
    wait_for(|| !alive(descendant));
    runner.shutdown().unwrap();
}
