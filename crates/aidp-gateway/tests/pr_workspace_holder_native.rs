//! Native runs record the pull request branch holder in the vault from their `pr` or `target` input.

use crate::native_runs::fake_provider;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aidp_agents::ProvidersConfig;
use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner};
use aidp_gateway::bus::Bus;
use aidp_gateway::flows::registry::AlwaysAllow;
use aidp_gateway::models::constants::pr_workspace_holder::{INPUT_PR, INPUT_TARGET};
use aidp_gateway::models::enums::native_run_status::NativeRunStatus;
use aidp_gateway::models::native_run_start::StartNativeRun;
use aidp_gateway::models::pr_workspace_holder::PrWorkspaceHolder;
use aidp_gateway::models::LocalLock;
use aidp_gateway::native_runs::{NativeDeps, NativeRunManager};
use aidp_gateway::pr_workspace_holder;
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::SessionManager;
use aidp_testkit::{EnvGuard, TempWorkspace};
use fake_provider::{Behaviour, Call, ScriptedProvider, PROVIDER_ID};
use serde_json::Value;

const TEMPLATE_PATH: &str = "tests/fixtures/native_runs/gated.yaml";
const VAULT_ENV: &str = "AIDP_VAULT";
const HOME_ENV: &str = "HOME";
const USER_PROFILE_ENV: &str = "USERPROFILE";
const README: &str = "README.md";
const README_CONTENT: &str = "original\n";
const FIRST_STAGE: &str = "draft";
const SECOND_STAGE: &str = "ship";
const ALL_GATES: &str = "*";
const GARBAGE_REPLY: &str = "this is not json";
const PR_NUMBER: u64 = 321;
const PR_URL: &str = "https://github.com/acme/svc/pull/321";
const HOLD: Duration = Duration::from_millis(1500);
const SETTLE: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(20);
const RUN_POLL: Duration = Duration::from_millis(10);

struct Harness {
    manager: Arc<NativeRunManager>,
    project_id: String,
    vault: PathBuf,
    workspace: PathBuf,
    _root: TempWorkspace,
    _env: EnvGuard,
}

fn run_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {arguments:?} failed");
}

fn init_workspace(directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(directory.join(README), README_CONTENT).unwrap();
    run_git(directory, &["init", "-q"]);
    run_git(directory, &["config", "user.email", "native@example.test"]);
    run_git(directory, &["config", "user.name", "native"]);
    run_git(directory, &["config", "core.autocrlf", "false"]);
    run_git(directory, &["add", "."]);
    run_git(directory, &["commit", "-q", "-m", "initial"]);
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

impl Harness {
    fn new(tag: &str, decide: impl Fn(&Call) -> Behaviour + Send + Sync + 'static) -> Harness {
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
        let runner: Arc<dyn ProcessRunner> = Arc::new(ScriptedProcessRunner::new());
        let provider = ScriptedProvider::new(None, decide);
        let sessions =
            SessionManager::load_with_runner(&state_dir, bus.clone(), projects.clone(), runner)
                .with_provider(PROVIDER_ID, provider)
                .with_providers_config(ProvidersConfig::default());
        let mut deps = NativeDeps::new(
            sessions,
            projects,
            bus,
            Arc::new(AlwaysAllow),
            Arc::new(LocalLock::default()),
            &state_dir,
            None,
        );
        deps.poll_interval = RUN_POLL;
        Harness {
            manager: NativeRunManager::new(deps),
            project_id,
            vault,
            workspace,
            _root: root,
            _env: env,
        }
    }

    fn start(&self, input: &str, value: &str) -> String {
        let template = Path::new(env!("CARGO_MANIFEST_DIR")).join(TEMPLATE_PATH);
        let request = StartNativeRun {
            project_id: self.project_id.clone(),
            template_id: template.to_string_lossy().to_string(),
            inputs: BTreeMap::from([(input.to_string(), Value::from(value))]),
            provider: Some(PROVIDER_ID.to_string()),
            auto_approve_gates: vec![ALL_GATES.to_string()],
            ..StartNativeRun::default()
        };
        self.manager.start(request).expect("the run starts")
    }

    fn start_with_target(&self, target: &str) -> String {
        let template = Path::new(env!("CARGO_MANIFEST_DIR")).join(TEMPLATE_PATH);
        let request = StartNativeRun {
            project_id: self.project_id.clone(),
            template_id: template.to_string_lossy().to_string(),
            target: Some(target.to_string()),
            provider: Some(PROVIDER_ID.to_string()),
            auto_approve_gates: vec![ALL_GATES.to_string()],
            ..StartNativeRun::default()
        };
        self.manager.start(request).expect("the run starts")
    }

    fn holder(&self) -> Option<PrWorkspaceHolder> {
        let segment = aidp_core::vault::repo_segment(&self.workspace);
        pr_workspace_holder::read(&self.vault, &segment, PR_NUMBER)
    }

    fn wait_for_holder_stage(&self, stage: &str) -> PrWorkspaceHolder {
        wait_until("the holder stage", || {
            self.holder().is_some_and(|holder| holder.stage == stage)
        });
        self.holder().unwrap()
    }
}

fn slow_stages(call: &Call) -> Behaviour {
    if call.label == FIRST_STAGE || call.label == SECOND_STAGE {
        Behaviour::ok().hold(HOLD)
    } else {
        Behaviour::ok()
    }
}

/// A `pr` input records the holder, follows the stage and clears the file when the run ends.
#[test]
fn it_a_run_with_a_pr_input_records_updates_and_removes_the_holder() {
    let harness = Harness::new("holder-native-pr", slow_stages);
    let run_id = harness.start(INPUT_PR, &PR_NUMBER.to_string());

    let first = harness.wait_for_holder_stage(FIRST_STAGE);
    assert_eq!(first.run_id.as_deref(), Some(run_id.as_str()));
    harness.wait_for_holder_stage(SECOND_STAGE);

    let view = harness.manager.wait_for(&run_id, SETTLE).unwrap();
    assert_eq!(view.status, NativeRunStatus::Done);
    assert_eq!(harness.holder(), None);
}

/// A `target` input naming a pull request URL records the holder too.
#[test]
fn it_a_run_with_a_pull_url_target_records_the_holder() {
    let harness = Harness::new("holder-native-target", slow_stages);
    let run_id = harness.start(INPUT_TARGET, PR_URL);

    harness.wait_for_holder_stage(FIRST_STAGE);

    harness.manager.wait_for(&run_id, SETTLE).unwrap();
    assert_eq!(harness.holder(), None);
}

/// A pull request given as the start target, not as an input, records the holder.
#[test]
fn it_a_run_started_with_a_pr_target_records_the_holder() {
    let harness = Harness::new("holder-native-start-target", slow_stages);
    let run_id = harness.start_with_target(&PR_NUMBER.to_string());

    let first = harness.wait_for_holder_stage(FIRST_STAGE);
    assert_eq!(first.run_id.as_deref(), Some(run_id.as_str()));

    harness.manager.wait_for(&run_id, SETTLE).unwrap();
    assert_eq!(harness.holder(), None);
}

/// A run whose stage fails still removes the holder file.
#[test]
fn it_a_failing_run_removes_the_holder() {
    let harness = Harness::new("holder-native-fail", |call| {
        if call.label == FIRST_STAGE {
            Behaviour::raw(GARBAGE_REPLY).hold(HOLD)
        } else {
            Behaviour::ok()
        }
    });
    let run_id = harness.start(INPUT_PR, &PR_NUMBER.to_string());

    harness.wait_for_holder_stage(FIRST_STAGE);

    let view = harness.manager.wait_for(&run_id, SETTLE).unwrap();
    assert_ne!(view.status, NativeRunStatus::Done);
    assert_eq!(harness.holder(), None);
}
