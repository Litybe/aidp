use super::*;
use crate::bus::Bus;
use crate::flows::registry::AlwaysAllow;
use crate::models::enums::agent_executor::AgentExecutor;
use crate::models::enums::WorkspaceKind;
use crate::models::workspace_request::WorkspaceRequest;
use crate::models::LocalLock;
use crate::native_runs::NativeDeps;
use crate::projects::Projects;
use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner};
use aidp_testkit::TempWorkspace;

const TEMPLATE_ID: &str = "no-such-template";
const TARGET: &str = "LOC-1";
const MODEL: &str = "fake-model";
const EFFORT: &str = "high";
const INPUT_NAME: &str = "scope";
const INPUT_VALUE: &str = "inline";
const FILE_INPUT_NAME: &str = "from_file";
const FILE_INPUT_VALUE: &str = "file";
const INPUTS_FILE: &str = "inputs.json";
const PIPELINE_NOT_FOUND: &str = "pipeline not found";
const PROJECT_NOT_FOUND: &str = "project not found";
const WORKTREE_TICKET: &str = "LOC-2";
const ENGINE_WORKFLOW_NAME: &str = "workflow";
const ENGINE_NATIVE_NAME: &str = "native";
const ENGINE_V7_NAME: &str = "v7";
const ENGINE_PROMPT_NAME: &str = "prompt";
const MISSING_PROJECT: &str = "ghost";
const PLACEHOLDER_PROJECT: &str = "p";
const RUN_ID: &str = "run-1";
const LEGACY_BODY: &str = r#"{"projectId":"p","templateId":"dev"}"#;
const MODEL_KEY: &str = "model";
const AUTO_APPROVE_ALL: &str = "*";
const AUTO_APPROVE_STAGE: &str = "plan";

struct Harness {
    launcher: RunLauncher,
    sessions: Arc<SessionManager>,
    project_id: String,
    workspace: TempWorkspace,
}

fn harness(name: &str) -> Harness {
    let workspace = TempWorkspace::new(name);
    let state_dir = workspace.child("state");
    let project_dir = workspace.child("project");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::create_dir_all(&project_dir).unwrap();
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&state_dir));
    let project_id = projects.add(project_dir.to_str().unwrap()).id;
    let runner: Arc<dyn ProcessRunner> = Arc::new(ScriptedProcessRunner::new());
    let sessions =
        SessionManager::load_with_runner(&state_dir, bus.clone(), projects.clone(), runner);
    let native = NativeRunManager::new(NativeDeps::new(
        sessions.clone(),
        projects,
        bus,
        Arc::new(AlwaysAllow),
        Arc::new(LocalLock::default()),
        &state_dir,
        None,
    ));
    Harness {
        launcher: RunLauncher::new(sessions.clone(), native),
        sessions,
        project_id,
        workspace,
    }
}

fn request(project_id: &str, engine: Option<&str>) -> StartRequest {
    StartRequest {
        project_id: project_id.to_string(),
        engine: engine.map(str::to_string),
        template_id: Some(TEMPLATE_ID.to_string()),
        target: Some(TARGET.to_string()),
        inputs: Some(BTreeMap::from([(
            INPUT_NAME.to_string(),
            INPUT_VALUE.to_string(),
        )])),
        model: Some(MODEL.to_string()),
        effort: Some(EFFORT.to_string()),
        ..StartRequest::default()
    }
}

fn launch_error(harness: &Harness, input: StartRequest) -> String {
    match harness.launcher.launch(input, false) {
        Ok(_) => panic!("the launch must fail inside the native manager"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn no_engine_with_a_template_goes_to_the_native_manager() {
    let harness = harness("launcher_native");
    let message = launch_error(&harness, request(&harness.project_id, None));
    assert!(message.contains(PIPELINE_NOT_FOUND), "{message}");
}

#[test]
fn native_engine_goes_to_the_native_manager() {
    let harness = harness("launcher_native_engine");
    let input = request(&harness.project_id, Some(ENGINE_NATIVE_NAME));
    let message = launch_error(&harness, input);
    assert!(message.contains(PIPELINE_NOT_FOUND), "{message}");
}

#[test]
fn native_launch_of_an_unknown_project_fails_in_the_native_manager() {
    let harness = harness("launcher_native_project");
    let message = launch_error(&harness, request(MISSING_PROJECT, None));
    assert!(message.contains(PROJECT_NOT_FOUND), "{message}");
}

#[test]
fn workflow_engine_returns_a_session_with_the_request_unchanged() {
    let harness = harness("launcher_workflow");
    let input = request(&harness.project_id, Some(ENGINE_WORKFLOW_NAME));
    let launched = harness
        .launcher
        .launch(input.clone(), true)
        .expect("the workflow session starts");
    let LaunchedRun::Session(launch) = launched else {
        panic!("a workflow engine must stay a session");
    };
    let record = harness
        .sessions
        .get(launch.id())
        .expect("the session is stored");
    assert_eq!(record.template_id, input.template_id);
    assert_eq!(record.target, input.target);
    assert_eq!(record.inputs, input.inputs);
    assert_eq!(record.model, input.model);
    assert_eq!(record.effort, input.effort);
}

#[test]
fn launched_run_exposes_only_the_id_of_its_kind() {
    let native = LaunchedRun::Native {
        run_id: RUN_ID.to_string(),
    };
    assert_eq!(native.run_id(), Some(RUN_ID));
    assert_eq!(native.session_id(), None);
}

#[test]
fn request_without_template_stays_a_session() {
    let mut input = request(PLACEHOLDER_PROJECT, None);
    input.template_id = None;
    assert!(stays_session(&input));
}

#[test]
fn workflow_v7_and_prompt_engines_stay_sessions() {
    for engine in [ENGINE_WORKFLOW_NAME, ENGINE_V7_NAME, ENGINE_PROMPT_NAME] {
        assert!(
            stays_session(&request(PLACEHOLDER_PROJECT, Some(engine))),
            "{engine}"
        );
    }
    assert!(!stays_session(&request(
        PLACEHOLDER_PROJECT,
        Some(ENGINE_NATIVE_NAME)
    )));
    assert!(!stays_session(&request(PLACEHOLDER_PROJECT, None)));
}

#[test]
fn autonomous_launch_approves_every_gate() {
    let mut input = request(PLACEHOLDER_PROJECT, None);
    input.auto_approve_gates = Some(vec![AUTO_APPROVE_STAGE.to_string()]);
    let native = native_request(input, TEMPLATE_ID.to_string(), true).unwrap();
    assert!(native.autonomous);
    assert_eq!(
        native.auto_approve_gates,
        vec![AUTO_APPROVE_ALL.to_string()]
    );
}

#[test]
fn non_autonomous_launch_keeps_the_requested_gates() {
    let mut input = request(PLACEHOLDER_PROJECT, None);
    input.auto_approve_gates = Some(vec![AUTO_APPROVE_STAGE.to_string()]);
    let native = native_request(input, TEMPLATE_ID.to_string(), false).unwrap();
    assert!(!native.autonomous);
    assert_eq!(
        native.auto_approve_gates,
        vec![AUTO_APPROVE_STAGE.to_string()]
    );
}

/// A launch the flow asked to run on a host becomes a native run that publishes agent jobs.
#[test]
fn native_request_carries_the_agent_executor() {
    let mut input = request(PLACEHOLDER_PROJECT, None);
    input.agent_executor = AgentExecutor::External;
    assert_eq!(
        native_request(input, TEMPLATE_ID.to_string(), false)
            .unwrap()
            .agent_executor,
        AgentExecutor::External
    );
    let plain = request(PLACEHOLDER_PROJECT, None);
    assert_eq!(
        native_request(plain, TEMPLATE_ID.to_string(), false)
            .unwrap()
            .agent_executor,
        AgentExecutor::Internal
    );
}

/// A request body never selects host-run sessions: only a flow launch sets the mode in process.
#[test]
fn a_start_request_body_cannot_select_external_agents() {
    let body = serde_json::json!({ "projectId": PLACEHOLDER_PROJECT, "agentExecutor": "external" });
    let parsed: StartRequest = serde_json::from_value(body).unwrap();
    assert_eq!(parsed.agent_executor, AgentExecutor::Internal);
}

/// A headless template launch becomes a headless native run; another one does not.
#[test]
fn native_request_keeps_the_headless_launch_policy() {
    let mut input = request(PLACEHOLDER_PROJECT, None);
    input.headless = true;
    assert!(
        native_request(input, TEMPLATE_ID.to_string(), false)
            .unwrap()
            .headless
    );
    let plain = request(PLACEHOLDER_PROJECT, None);
    assert!(
        !native_request(plain, TEMPLATE_ID.to_string(), false)
            .unwrap()
            .headless
    );
}

#[test]
fn native_request_carries_model_effort_target_and_worktree_ticket() {
    let mut input = request(PLACEHOLDER_PROJECT, None);
    input.worktree = Some(WORKTREE_TICKET.to_string());
    let native = native_request(input, TEMPLATE_ID.to_string(), false).unwrap();
    assert_eq!(native.model.as_deref(), Some(MODEL));
    assert_eq!(native.effort.as_deref(), Some(EFFORT));
    assert_eq!(native.target.as_deref(), Some(TARGET));
    assert_eq!(native.ticket.as_deref(), Some(WORKTREE_TICKET));
}

#[test]
fn workspace_ticket_wins_over_the_worktree() {
    let mut input = request(PLACEHOLDER_PROJECT, None);
    input.worktree = Some(WORKTREE_TICKET.to_string());
    input.workspace = Some(WorkspaceRequest {
        kind: WorkspaceKind::Ticket,
        ticket: Some(TARGET.to_string()),
        name: None,
        path: None,
    });
    let native = native_request(input, TEMPLATE_ID.to_string(), false).unwrap();
    assert_eq!(native.ticket.as_deref(), Some(TARGET));
}

#[test]
fn inline_inputs_override_the_inputs_file() {
    let harness = harness("launcher_inputs_file");
    let file = harness.workspace.child(INPUTS_FILE);
    let content = format!(
        r#"{{"{INPUT_NAME}":"{FILE_INPUT_VALUE}","{FILE_INPUT_NAME}":"{FILE_INPUT_VALUE}"}}"#
    );
    std::fs::write(&file, content).unwrap();
    let mut input = request(PLACEHOLDER_PROJECT, None);
    input.inputs_file = Some(file.to_string_lossy().to_string());
    let merged = merged_inputs(&input).unwrap();
    assert_eq!(merged[INPUT_NAME], Value::String(INPUT_VALUE.to_string()));
    assert_eq!(
        merged[FILE_INPUT_NAME],
        Value::String(FILE_INPUT_VALUE.to_string())
    );
}

#[test]
fn start_native_run_without_the_optional_fields_still_deserializes() {
    let parsed: StartNativeRun = serde_json::from_str(LEGACY_BODY).unwrap();
    assert!(parsed.origin.is_none());
    assert!(parsed.model.is_none());
    assert!(parsed.permission_mode.is_none());
    assert!(parsed.effort.is_none());
    assert!(parsed.ticket.is_none());
    let serialized = serde_json::to_value(&parsed).unwrap();
    assert!(serialized.get(MODEL_KEY).is_none());
}
