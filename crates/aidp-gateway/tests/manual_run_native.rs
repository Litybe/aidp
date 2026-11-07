//! `POST /api/runs/start`: a pipeline run with no ticket, driven by a scripted provider.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::native_runs::fake_provider::{Behaviour, ScriptedProvider, PROVIDER_ID};
use aidp_agents::ProvidersConfig;
use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner};
use aidp_gateway::bus::Bus;
use aidp_gateway::flows::registry::AlwaysAllow;
use aidp_gateway::manual_run::start_run;
use aidp_gateway::models::enums::native_run_status::NativeRunStatus;
use aidp_gateway::models::LocalLock;
use aidp_gateway::native_runs::{NativeDeps, NativeRunManager};
use aidp_gateway::projects::{Project, Projects};
use aidp_gateway::sessions::SessionManager;
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde_json::{json, Value};

const TEMPLATE_ID: &str = "manual-demo";
const FRAGMENT_ID: &str = "manual-brick";
const TEMPLATE: &str = "version: 5
id: manual-demo
kind: pipeline
description: One stage, one required input.
inputs:
  topic: { required: true, kind: string, description: What to write about }
  mood: { required: false, kind: string, default: calm }
  track: { required: false, kind: string, default: standard }
stages:
  - name: draft
    description: Write the draft.
    agent: analyzer
    provider: fake
    outputs: [draft.md]
";
const FRAGMENT: &str = "version: 5
id: manual-brick
kind: fragment
description: A brick, never launched alone.
stages:
  - name: brick
    description: Brick.
    agent: analyzer
    outputs: [brick.md]
";
const VAULT_ENV: &str = "AIDP_VAULT";
const HOME_ENV: &str = "HOME";
const USER_PROFILE_ENV: &str = "USERPROFILE";
const STATUS_ACCEPTED: u16 = 202;
const STATUS_BAD_REQUEST: u16 = 400;
const SETTLE: Duration = Duration::from_secs(45);

struct Fixture {
    manager: Arc<NativeRunManager>,
    bus: Arc<Bus>,
    project: Project,
    _root: TempWorkspace,
    _env: EnvGuard,
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {arguments:?} failed");
}

fn fixture(tag: &str) -> Fixture {
    let root = TempWorkspace::new(tag);
    let vault = root.child("vault");
    let home = root.child("home");
    let workspace = root.child("workspace");
    let state_dir = root.child("state");
    let skill = root.child("skill");
    for directory in [
        &vault,
        &home,
        &state_dir,
        &workspace,
        &skill.join("templates"),
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    std::fs::write(skill.join("templates").join("manual-demo.yaml"), TEMPLATE).unwrap();
    std::fs::write(skill.join("templates").join("manual-brick.yaml"), FRAGMENT).unwrap();
    std::fs::write(workspace.join("README.md"), "demo\n").unwrap();
    git(&workspace, &["init", "-q"]);
    git(&workspace, &["config", "user.email", "manual@example.test"]);
    git(&workspace, &["config", "user.name", "manual"]);
    git(&workspace, &["config", "core.autocrlf", "false"]);
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-q", "-m", "initial"]);
    let env = EnvGuard::with(&[
        (VAULT_ENV, Some(vault.to_str().unwrap())),
        (HOME_ENV, Some(home.to_str().unwrap())),
        (USER_PROFILE_ENV, Some(home.to_str().unwrap())),
    ]);
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(workspace.to_str().unwrap());
    let bus = Arc::new(Bus::new());
    let provider = ScriptedProvider::new(None, |_| Behaviour::ok());
    let runner: Arc<dyn ProcessRunner> = Arc::new(ScriptedProcessRunner::new());
    let sessions =
        SessionManager::load_with_runner(&state_dir, bus.clone(), projects.clone(), runner)
            .with_provider(PROVIDER_ID, provider)
            .with_providers_config(ProvidersConfig::default());
    let mut deps = NativeDeps::new(
        sessions,
        projects,
        bus.clone(),
        Arc::new(AlwaysAllow),
        Arc::new(LocalLock::default()),
        &state_dir,
        Some(skill),
    );
    deps.poll_interval = Duration::from_millis(10);
    Fixture {
        manager: NativeRunManager::new(deps),
        bus,
        project,
        _root: root,
        _env: env,
    }
}

fn refused(fixture: &Fixture, body: Value) -> (u16, String) {
    let (status, reply) = start_run(&fixture.manager, &fixture.project, &body);
    (
        status,
        reply["error"].as_str().unwrap_or_default().to_string(),
    )
}

#[test]
fn a_ticketless_run_starts_finishes_and_announces_itself() {
    let fixture = fixture("manual-run-ok");
    let body = json!({
        "template": TEMPLATE_ID,
        "inputs": { "topic": "frogs" },
        "track": "quick",
    });
    let (status, reply) = start_run(&fixture.manager, &fixture.project, &body);
    assert_eq!(status, STATUS_ACCEPTED, "{reply}");
    let run_id = reply["runId"].as_str().expect("runId").to_string();
    assert!(
        reply.get("sessionId").is_none(),
        "a native run has no session: {reply}"
    );

    let view = fixture
        .manager
        .wait_for(&run_id, SETTLE)
        .expect("the run exists");
    assert_eq!(view.status, NativeRunStatus::Done);
    assert_eq!(view.request.project_id, fixture.project.id);
    assert_eq!(view.request.template_id, TEMPLATE_ID);
    assert_eq!(view.request.inputs["topic"], json!("frogs"));
    assert_eq!(view.request.inputs["track"], json!("quick"));
    assert!(view.request.ticket.is_none());

    let started = fixture
        .bus
        .recent(100, 0)
        .into_iter()
        .any(|event| event["type"] == "run.started");
    assert!(started, "the usual run.started event is emitted");
}

#[test]
fn a_ticketless_run_is_refused_with_400_on_a_bad_request() {
    let fixture = fixture("manual-run-400");
    let cases = [
        (json!({}), "template is required"),
        (json!({ "template": "  " }), "template is required"),
        (json!({ "template": "nope" }), "unknown template `nope`"),
        (
            json!({ "template": TEMPLATE_ID }),
            "required input 'topic' not provided",
        ),
        (
            json!({ "template": TEMPLATE_ID, "inputs": ["topic"] }),
            "inputs must be an object",
        ),
        (
            json!({ "template": TEMPLATE_ID, "inputs": { "topic": "x" }, "track": "bogus" }),
            "bogus",
        ),
        (json!({ "template": FRAGMENT_ID }), "is a fragment"),
    ];
    for (body, expected) in cases {
        let (status, message) = refused(&fixture, body.clone());
        assert_eq!(status, STATUS_BAD_REQUEST, "{body}: {message}");
        assert!(message.contains(expected), "{body}: {message}");
    }
    assert!(
        fixture.manager.list().is_empty(),
        "no refused request starts a run"
    );
}
