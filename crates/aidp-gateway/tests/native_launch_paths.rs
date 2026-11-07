//! Every daemon launch path creates a native run; the workflow engine stays a session.

use crate::native_runs::fake_provider;
#[path = "common_native/mcp_recording_provider.rs"]
mod mcp_recording_provider;
use mcp_recording_provider::McpRecordingProvider;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aidp_agents::{ProviderEntry, ProvidersConfig};
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner};
use aidp_gateway::automations::AutomationEngine;
use aidp_gateway::bus::Bus;
use aidp_gateway::claude_provider::CLAUDE_PROVIDER_ID;
use aidp_gateway::dispatcher::Dispatcher;
use aidp_gateway::flows::registry::{
    AlwaysAllow, ManagerSessions, SessionPort, SessionStart, SessionState,
};
use aidp_gateway::launcher::RunLauncher;
use aidp_gateway::models::constants::engine::{ENGINE_NATIVE, ENGINE_WORKFLOW};
use aidp_gateway::models::enums::agent_executor::AgentExecutor;
use aidp_gateway::models::enums::native_run_status::NativeRunStatus;
use aidp_gateway::models::launched_run::LaunchedRun;
use aidp_gateway::models::native_run_view::NativeRunView;
use aidp_gateway::models::{LocalLock, PipelineLaunch, ProjectRef};
use aidp_gateway::native_runs::{NativeDeps, NativeRunManager};
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::{Launch, SessionManager, StartRequest};
use aidp_gateway::worktrees::worktree_path;
use aidp_testkit::{EnvGuard, TempWorkspace};
use aidp_tickets::lifecycle::{self, PrState};
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::store::TicketStore;
use fake_provider::{Behaviour, ScriptedProvider, PROVIDER_ID};
use serde_json::{json, Value};

const MCP_CONFIGURATION_ID: &str = "launch-paths";
const MCP_ENVIRONMENT_KEY: &str = "PRIVATE_MCP_VALUE";
const MCP_SECRET: &str = "runtime-only-private-value";

const TEMPLATE_SOURCE: &str = include_str!("fixtures/autonomy/launch_paths.yaml");
const TEMPLATE_ID: &str = "native-autonomous";
const TEMPLATES_DIR: &str = "templates";
const TEMPLATE_EXTENSION: &str = "yaml";
const VAULT_ENV: &str = "AIDP_VAULT";
const HOME_ENV: &str = "HOME";
const USER_PROFILE_ENV: &str = "USERPROFILE";
const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";
const CLAUDE_BIN_ENV: &str = "AIDP_CLAUDE_BIN";
const FEATURE_INPUT: &str = "feature";
const FEATURE_VALUE: &str = "launch-paths";
const AIDP_DIR: &str = ".aidp";
const CONFIG_FILE: &str = "config.json";
const PROVIDERS_KEY: &str = "providers";
const INDEX_NOTE_PREFIX: &str = "_";
const RUN_STATE_FILE: &str = "run-state.json";
const MODE_KEY: &str = "mode";
const ORIGINAL_README: &str = "original\n";
const README: &str = "README.md";
const ORIGIN_DIR: &str = "origin.git";
const ORIGIN_REMOTE: &str = "origin";
const GIT_CLONE: &str = "clone";
const GIT_QUIET: &str = "-q";
const GIT_BARE: &str = "--bare";
const GIT_REMOTE: &str = "remote";
const GIT_ADD: &str = "add";
const GIT_FETCH: &str = "fetch";
const CURRENT_DIR: &str = ".";
const TICKET_KEY: &str = "TEST-1";
const PR_DIR: &str = "pr";
const PR_FILE: &str = "pr.json";
const PR_URL: &str = "https://example.test/pull/7";
const PR_NUMBER: u64 = 7;
const PR_OPEN_STATE: &str = "OPEN";
const AUTOMATION_NAME: &str = "launch-paths";
const FLOW_KEY: &str = "FLOW-1";
const FLOW_STEP: &str = "develop";
const FLOW_INPUTS_FILE: &str = "flow-inputs.json";
const OUTPUT_RUN_ID: &str = "runId";
const OUTPUT_RUN_DIR: &str = "runDir";
const SESSION_STATE_DONE: &str = "done";
const SETTLE: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(20);
const RUN_POLL: Duration = Duration::from_millis(10);
const STUB_SLEEP_SECONDS: u32 = 30;

struct Stack {
    mcp_provider: Arc<McpRecordingProvider>,
    launcher: Arc<RunLauncher>,
    sessions: Arc<SessionManager>,
    projects: Arc<Projects>,
    bus: Arc<Bus>,
    project_id: String,
    project_name: String,
    state_dir: PathBuf,
    vault: PathBuf,
    workspace: PathBuf,
    _root: TempWorkspace,
    _env: EnvGuard,
}

/// Runs git in `directory` and fails the test when it fails.
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

/// The canonical form of an existing directory, so two spellings of one path compare equal.
fn same_dir(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).expect("the directory exists")
}

/// A committed git workspace holding one file, with a bare `origin` it was fetched from:
/// a ticket worktree branches off `origin/<default>`.
fn init_workspace(directory: &Path, origin: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(directory.join(README), ORIGINAL_README).unwrap();
    run_git(directory, &["init", "-q"]);
    run_git(directory, &["config", "user.email", "native@example.test"]);
    run_git(directory, &["config", "user.name", "native"]);
    run_git(directory, &["config", "core.autocrlf", "false"]);
    run_git(directory, &["add", "."]);
    run_git(directory, &["commit", "-q", "-m", "initial"]);
    let origin_arg = origin.to_string_lossy().to_string();
    run_git(
        directory,
        &[GIT_CLONE, GIT_QUIET, GIT_BARE, CURRENT_DIR, &origin_arg],
    );
    run_git(
        directory,
        &[GIT_REMOTE, GIT_ADD, ORIGIN_REMOTE, &origin_arg],
    );
    run_git(directory, &[GIT_FETCH, GIT_QUIET, ORIGIN_REMOTE]);
}

/// A stub `claude` that stays alive without doing anything.
fn stub_claude(directory: &Path) -> PathBuf {
    if cfg!(windows) {
        let binary = directory.join("claude.cmd");
        let script =
            format!("@echo off\r\nping -n {STUB_SLEEP_SECONDS} 127.0.0.1 >nul\r\nexit /b 0\r\n");
        std::fs::write(&binary, script).unwrap();
        return binary;
    }
    let binary = directory.join("claude");
    std::fs::write(
        &binary,
        format!("#!/bin/sh\nsleep {STUB_SLEEP_SECONDS}\nexit 0\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    binary
}

/// Writes the providers config the native executor reads from the home directory.
fn write_providers_config(home: &Path, config: &ProvidersConfig) {
    let directory = home.join(AIDP_DIR);
    std::fs::create_dir_all(&directory).unwrap();
    let body = json!({ PROVIDERS_KEY: config });
    std::fs::write(directory.join(CONFIG_FILE), body.to_string()).unwrap();
}

/// The in-process daemon: sessions, native manager and launcher over a fake provider.
fn build_stack(tag: &str) -> Stack {
    let root = TempWorkspace::new(tag);
    let vault = root.child("vault");
    let home = root.child("home");
    let workspace = root.child("workspace");
    let state_dir = root.child("state");
    let skill_dir = root.child("skill");
    let bin_dir = root.child("bin");
    let templates_dir = skill_dir.join(TEMPLATES_DIR);
    for directory in [&vault, &home, &state_dir, &templates_dir, &bin_dir] {
        std::fs::create_dir_all(directory).unwrap();
    }
    std::fs::write(
        templates_dir.join(format!("{TEMPLATE_ID}.{TEMPLATE_EXTENSION}")),
        TEMPLATE_SOURCE,
    )
    .unwrap();
    init_workspace(&workspace, &root.child(ORIGIN_DIR));
    let claude = stub_claude(&bin_dir);
    let env = EnvGuard::with(&[
        (VAULT_ENV, Some(vault.to_str().unwrap())),
        (HOME_ENV, Some(home.to_str().unwrap())),
        (USER_PROFILE_ENV, Some(home.to_str().unwrap())),
        (SKILL_ROOT_ENV, Some(skill_dir.to_str().unwrap())),
        (CLAUDE_BIN_ENV, Some(claude.to_str().unwrap())),
    ]);
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(workspace.to_str().unwrap());
    let bus = Arc::new(Bus::new());
    let provider = McpRecordingProvider::new(ScriptedProvider::new(None, |_| Behaviour::ok()));
    let runner: Arc<dyn ProcessRunner> = Arc::new(OsProcessRunner);
    let config = ProvidersConfig {
        default: Some(PROVIDER_ID.to_string()),
        entries: BTreeMap::from([(PROVIDER_ID.to_string(), ProviderEntry::default())]),
    };
    write_providers_config(&home, &config);
    let sessions =
        SessionManager::load_with_runner(&state_dir, bus.clone(), projects.clone(), runner)
            .with_provider(PROVIDER_ID, provider.clone())
            .with_providers_config(config);
    let mut deps = NativeDeps::new(
        sessions.clone(),
        projects.clone(),
        bus.clone(),
        Arc::new(AlwaysAllow),
        Arc::new(LocalLock::default()),
        &state_dir,
        Some(skill_dir),
    );
    deps.poll_interval = RUN_POLL;
    let launcher = Arc::new(RunLauncher::new(
        sessions.clone(),
        NativeRunManager::new(deps),
    ));
    Stack {
        mcp_provider: provider,
        launcher,
        sessions,
        projects,
        bus,
        project_id: project.id,
        project_name: project.name,
        state_dir,
        vault,
        workspace,
        _root: root,
        _env: env,
    }
}

impl Stack {
    /// A template launch of the fixture pipeline, with no engine named.
    fn template_request(&self) -> StartRequest {
        StartRequest {
            project_id: self.project_id.clone(),
            template_id: Some(TEMPLATE_ID.to_string()),
            inputs: Some(BTreeMap::from([(
                FEATURE_INPUT.to_string(),
                FEATURE_VALUE.to_string(),
            )])),
            ..StartRequest::default()
        }
    }

    /// The native run `run_id`, once it stopped executing.
    fn finish(&self, run_id: &str) -> NativeRunView {
        self.launcher
            .wait_for(run_id, SETTLE)
            .expect("the native run exists")
    }

    /// The parsed `run-state.json` of a native run.
    fn state_json(&self, view: &NativeRunView) -> Value {
        let text = std::fs::read_to_string(view.run_dir.join(RUN_STATE_FILE)).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn dispatcher(&self) -> Arc<Dispatcher> {
        Dispatcher::new_with_launcher(
            self.vault.clone(),
            self.sessions.clone(),
            self.launcher.clone(),
            self.projects.clone(),
            self.bus.clone(),
        )
    }

    fn automations(&self) -> Arc<AutomationEngine> {
        let engine = AutomationEngine::load(
            &self.state_dir,
            self.sessions.clone(),
            self.projects.clone(),
            self.bus.clone(),
        );
        engine.set_launcher(self.launcher.clone());
        engine
    }
}

/// The run id of a launch that must have become a native run.
fn native_run_id(launched: LaunchedRun) -> String {
    match launched {
        LaunchedRun::Native { run_id } => run_id,
        LaunchedRun::Session(launch) => {
            panic!("expected a native run, got session {}", launch.id())
        }
    }
}

/// Polls `condition` until it holds or the settle deadline passes.
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

/// A manual-priority todo ticket filed under `service`, using the fixture template.
fn seed_ticket(vault: &Path, service: &str) {
    let now = chrono::Utc::now();
    let mut ticket = Ticket {
        key: TICKET_KEY.to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "launch paths".to_string(),
        status: Status::Todo,
        priority: Priority::Medium,
        priority_rank: Priority::Medium.rank(),
        service: service.to_string(),
        template: Some(TEMPLATE_ID.to_string()),
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created: now,
        updated: now,
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot: Autopilot {
            manual_priority: Some(true),
            ..Autopilot::default()
        },
        tags: Vec::new(),
    };
    TicketStore::open(vault)
        .unwrap()
        .save(&mut ticket, "do the work", &serde_yaml::Mapping::new())
        .unwrap();
}

/// The stored ticket.
fn stored_ticket(vault: &Path) -> Ticket {
    TicketStore::open(vault).unwrap().get(TICKET_KEY).unwrap().0
}

/// Writes the pull request file a `pr` stage leaves in the run directory.
fn write_pr_file(run_dir: &Path) {
    let directory = run_dir.join(PR_DIR);
    std::fs::create_dir_all(&directory).unwrap();
    let body = json!({ "url": PR_URL, "number": PR_NUMBER, "state": PR_OPEN_STATE });
    std::fs::write(directory.join(PR_FILE), body.to_string()).unwrap();
}

/// File names, not directories or generated index notes, directly under `directory`.
fn file_names(directory: &Path) -> BTreeSet<String> {
    std::fs::read_dir(directory)
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with(INDEX_NOTE_PREFIX))
        .collect()
}

#[test]
fn session_start_with_a_template_id_creates_a_native_run() {
    let stack = build_stack("launch-paths-session");

    let run_id = native_run_id(
        stack
            .launcher
            .launch(stack.template_request(), false)
            .unwrap(),
    );
    let view = stack.finish(&run_id);

    assert_eq!(view.status, NativeRunStatus::Done);
    assert_eq!(stack.state_json(&view)[MODE_KEY], ENGINE_NATIVE);
    assert!(!view.autonomous);
}

#[test]
fn ticket_start_creates_a_native_run_and_follows_the_pull_request_to_done() {
    let stack = build_stack("launch-paths-ticket");
    seed_ticket(&stack.vault, &stack.project_name);
    let dispatcher = stack.dispatcher();

    let report = dispatcher.tick_local("test");

    assert_eq!(report["launched"], json!([TICKET_KEY]), "{report}");
    let started = stored_ticket(&stack.vault);
    assert_eq!(started.status, Status::InProgress);
    let run_id = started
        .autopilot
        .run_id
        .clone()
        .expect("the run id is recorded");
    let view = stack.finish(&run_id);
    assert_eq!(stack.state_json(&view)[MODE_KEY], ENGINE_NATIVE);
    let worktree = worktree_path(
        &stack.vault,
        &aidp_core::vault::repo_segment(&stack.workspace),
        TICKET_KEY,
    );
    assert_eq!(same_dir(&view.workspace), same_dir(&worktree));

    write_pr_file(&view.run_dir);
    dispatcher.tick("probe");
    assert_eq!(stored_ticket(&stack.vault).status, Status::InReview);

    let store = TicketStore::open(&stack.vault).unwrap();
    let (mut ticket, body, extra) = store.get(TICKET_KEY).unwrap();
    lifecycle::on_pr_state(&mut ticket, PrState::Merged).expect("a merge moves the ticket");
    store.save(&mut ticket, &body, &extra).unwrap();
    assert_eq!(stored_ticket(&stack.vault).status, Status::Done);
}

#[test]
fn automation_trigger_creates_an_autonomous_native_run() {
    let stack = build_stack("launch-paths-automation");
    let engine = stack.automations();
    let automation = engine
        .upsert(json!({
            "name": AUTOMATION_NAME,
            "trigger": { "kind": "webhook" },
            "action": {
                "kind": "launch",
                "projectId": stack.project_id,
                "templateId": TEMPLATE_ID,
                "inputs": { FEATURE_INPUT: FEATURE_VALUE },
                "autonomous": true
            }
        }))
        .unwrap();

    let run = engine
        .run_automation(&automation.id, "test", json!({}))
        .unwrap();

    let run_id = run.session_id.expect("the native run id is recorded");
    let view = stack.finish(&run_id);
    assert_eq!(view.status, NativeRunStatus::Done);
    assert!(view.autonomous);
}

#[test]
fn flow_pipeline_step_awaits_a_native_run_and_reads_its_outputs() {
    let stack = build_stack("launch-paths-flow");
    stack
        .sessions
        .mcp_configurations()
        .upsert(
            MCP_CONFIGURATION_ID.to_string(),
            aidp_agents::McpConfiguration {
                servers: Vec::new(),
                environment: BTreeMap::from([(
                    MCP_ENVIRONMENT_KEY.to_string(),
                    MCP_SECRET.to_string(),
                )]),
            },
        )
        .unwrap();
    let inputs_file = stack.state_dir.join(FLOW_INPUTS_FILE);
    std::fs::write(
        &inputs_file,
        json!({ FEATURE_INPUT: FEATURE_VALUE }).to_string(),
    )
    .unwrap();
    let port = ManagerSessions::new(
        stack.sessions.clone(),
        stack.projects.clone(),
        stack.launcher.clone(),
    );
    let launch = PipelineLaunch {
        key: FLOW_KEY.to_string(),
        ticket_key: FLOW_KEY.to_string(),
        provider: None,
        permission_mode: None,
        strict_mcp_config: None,
        mcp_config_id: Some(MCP_CONFIGURATION_ID.to_string()),
        mcp_servers: None,
        denied_shell_commands: Vec::new(),
        denied_mcp_tools: BTreeMap::new(),
        step: FLOW_STEP.to_string(),
        template: TEMPLATE_ID.to_string(),
        project: ProjectRef {
            id: stack.project_id.clone(),
            path: stack.workspace.clone(),
            repo: aidp_core::vault::repo_segment(&stack.workspace),
        },
        inputs_file,
        auto_approve_gates: None,
        autonomous: false,
        headless: false,
        engine: None,
        agent_executor: AgentExecutor::Internal,
    };

    let SessionStart::Started(run_id) = port.start(&launch).unwrap() else {
        panic!("the flow step must start");
    };

    let mut ended = None;
    wait_until("the flow step to end", || match port.state(&run_id) {
        SessionState::Running => false,
        other => {
            ended = Some(other);
            true
        }
    });
    let Some(SessionState::Ended {
        run_status,
        outputs,
    }) = ended
    else {
        panic!("the flow step must end");
    };
    assert_eq!(run_status, SESSION_STATE_DONE);
    assert_eq!(outputs[OUTPUT_RUN_ID], json!(run_id));
    let view = stack.finish(&run_id);
    assert_eq!(
        view.request.mcp_config_id.as_deref(),
        Some(MCP_CONFIGURATION_ID)
    );
    let snapshots = stack.mcp_provider.configurations();
    assert!(!snapshots.is_empty());
    for configuration in snapshots {
        assert_eq!(
            configuration.unwrap().environment[MCP_ENVIRONMENT_KEY],
            MCP_SECRET
        );
    }
    assert!(!serde_json::to_string(&view).unwrap().contains(MCP_SECRET));
    assert_private_value_absent(&stack.state_dir);
    assert_private_value_absent(&stack.vault);
    let worktree = worktree_path(
        &stack.vault,
        &aidp_core::vault::repo_segment(&stack.workspace),
        FLOW_KEY,
    );
    assert_eq!(
        same_dir(&view.workspace),
        same_dir(&worktree),
        "a flow step runs in the ticket worktree, never in the checkout"
    );
    assert_eq!(
        outputs[OUTPUT_RUN_DIR],
        json!(view.run_dir.to_string_lossy())
    );
}

#[test]
fn workflow_engine_stays_a_session_and_matches_the_native_artifact_names() {
    let stack = build_stack("launch-paths-workflow");
    let request = StartRequest {
        engine: Some(ENGINE_WORKFLOW.to_string()),
        provider: Some(CLAUDE_PROVIDER_ID.to_string()),
        ..stack.template_request()
    };

    let launched = stack.launcher.launch(request, false).unwrap();

    let LaunchedRun::Session(Launch::Started(record)) = launched else {
        panic!("the workflow engine must start a session");
    };
    assert_eq!(record.engine, ENGINE_WORKFLOW);
    let _ = stack.sessions.cancel(&record.id);

    let native_id = native_run_id(
        stack
            .launcher
            .launch(stack.template_request(), false)
            .unwrap(),
    );
    let native_view = stack.finish(&native_id);
    let template = stack
        .workspace
        .join(format!("{TEMPLATE_ID}.{TEMPLATE_EXTENSION}"));
    std::fs::write(&template, TEMPLATE_SOURCE).unwrap();
    let vault = aidp_core::vault::resolve_global();
    let workflow_state = aidp_core::run::init(
        &template,
        &stack.workspace,
        json!({ FEATURE_INPUT: FEATURE_VALUE }),
        vault.as_ref(),
    )
    .expect("workflow-mode init");

    let workflow_state_json: Value = serde_json::from_str(
        &std::fs::read_to_string(workflow_state.run_dir.join(RUN_STATE_FILE)).unwrap(),
    )
    .unwrap();
    assert_ne!(workflow_state_json[MODE_KEY], ENGINE_NATIVE);
    assert_eq!(stack.state_json(&native_view)[MODE_KEY], ENGINE_NATIVE);
    let native_names = file_names(&native_view.run_dir);
    let workflow_names = file_names(&workflow_state.run_dir);
    let missing: Vec<_> = workflow_names.difference(&native_names).collect();
    assert!(
        missing.is_empty(),
        "artifacts lost in native mode: {missing:?}"
    );
}

fn assert_private_value_absent(directory: &Path) {
    for entry in std::fs::read_dir(directory).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            assert_private_value_absent(&path);
        } else {
            assert!(!std::fs::read(&path)
                .unwrap()
                .windows(MCP_SECRET.len())
                .any(|bytes| bytes == MCP_SECRET.as_bytes()));
        }
    }
}
