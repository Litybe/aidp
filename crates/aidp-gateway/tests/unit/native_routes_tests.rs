use super::*;
use crate::bus::Bus;
use crate::flows::registry::AlwaysAllow;
use crate::models::LocalLock;
use crate::native_runs::NativeDeps;
use crate::projects::Projects;
use crate::sessions::SessionManager;
use serde::Serialize;

const BAD_REQUEST: u16 = 400;
const TEMPLATE_DEV: &str = "dev";
const PROJECT_ID: &str = "p";
const GHOST_PROJECT: &str = "ghost";
const RUNS_PATH: &str = "/api/runs";
const RELATIVE_WORKSPACE: &str = "worktree";
const OWN_VAULT: &str = "own-vault";
const CLIENT_REQUEST_ID: &str = "ada-native-91";
const UNSUPPORTED_MCP_DEFINITION: &str = "unsupported-definition";
const HOME_ENV: &str = "HOME";
const USERPROFILE_ENV: &str = "USERPROFILE";

const MISSING_MCP_CONFIGURATION: &str = "missing-runtime-configuration";

#[test]
fn unavailable_mcp_configuration_is_rejected_without_creating_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(dir.path());
    let body = serde_json::to_value(&NativeRunRequest {
        project_id: PROJECT_ID.to_string(),
        template_id: TEMPLATE_DEV.to_string(),
        mcp_config_id: Some(MISSING_MCP_CONFIGURATION.to_string()),
        ..Default::default()
    })
    .unwrap();
    let reply = handle(&manager, &Method::Post, RUNS_PATH, &body).unwrap();
    assert_eq!(reply.status, BAD_REQUEST);
    assert!(manager.list().is_empty());
}

#[test]
fn unsupported_mcp_definitions_are_rejected_without_creating_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(dir.path());
    let body = serde_json::to_value(&NativeRunRequest {
        project_id: PROJECT_ID.to_string(),
        template_id: TEMPLATE_DEV.to_string(),
        mcp_servers: Some(vec![Value::String(UNSUPPORTED_MCP_DEFINITION.to_string())]),
        ..Default::default()
    })
    .unwrap();
    let reply = handle(&manager, &Method::Post, RUNS_PATH, &body).unwrap();
    assert_eq!(reply.status, BAD_REQUEST);
    assert!(manager.list().is_empty());
}

#[test]
fn client_request_identity_and_permission_survive_request_parsing() {
    let body = serde_json::to_value(&NativeRunRequest {
        project_id: PROJECT_ID.to_string(),
        template_id: TEMPLATE_DEV.to_string(),
        client_request_id: Some(CLIENT_REQUEST_ID.to_string()),
        permission_mode: Some(
            crate::models::enums::PermissionMode::DontAsk
                .as_str()
                .to_string(),
        ),
        ..Default::default()
    })
    .unwrap();
    let request = parse_request(&body).unwrap();
    assert_eq!(
        request.client_request_id.as_deref(),
        Some(CLIENT_REQUEST_ID)
    );
    assert_eq!(
        request.permission_mode.as_deref(),
        Some(crate::models::enums::PermissionMode::DontAsk.as_str())
    );
}

#[test]
fn blank_client_request_identity_is_rejected() {
    let body = serde_json::to_value(&NativeRunRequest {
        project_id: PROJECT_ID.to_string(),
        template_id: TEMPLATE_DEV.to_string(),
        client_request_id: Some(String::new()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        call(Method::Post, RUNS_PATH, body).unwrap().status,
        BAD_REQUEST
    );
}

/// Body whose `inputs` has the wrong type.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WrongShapeBody {
    template_id: String,
    inputs: i32,
}

fn body_of(project_id: &str, template_id: &str) -> Value {
    serde_json::to_value(&NativeRunRequest {
        project_id: project_id.to_string(),
        template_id: template_id.to_string(),
        ..NativeRunRequest::default()
    })
    .unwrap()
}

const NOT_FOUND: u16 = 404;
const CONFLICT: u16 = 409;

fn manager(dir: &std::path::Path) -> Arc<NativeRunManager> {
    let _environment = aidp_testkit::EnvGuard::with(&[
        (HOME_ENV, Some(dir.to_str().unwrap())),
        (USERPROFILE_ENV, Some(dir.to_str().unwrap())),
        (
            crate::models::constants::flow_execution::VAULT_ENV,
            Some(crate::models::constants::flow_execution::VAULT_DISABLED),
        ),
    ]);
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(dir));
    let sessions = SessionManager::load(dir, bus.clone(), projects.clone());
    NativeRunManager::new(NativeDeps::new(
        sessions,
        projects,
        bus,
        Arc::new(AlwaysAllow),
        Arc::new(LocalLock::default()),
        dir,
        None,
    ))
}

fn call(method: Method, path: &str, body: Value) -> Option<FlowReply> {
    let dir = tempfile::tempdir().unwrap();
    handle(&manager(dir.path()), &method, path, &body)
}

#[test]
fn body_without_project_is_a_bad_request() {
    let reply = call(Method::Post, RUNS_PATH, body_of("", TEMPLATE_DEV)).unwrap();
    assert_eq!(reply.status, BAD_REQUEST);
}

#[test]
fn body_without_template_is_a_bad_request() {
    let reply = call(Method::Post, RUNS_PATH, body_of(PROJECT_ID, "")).unwrap();
    assert_eq!(reply.status, BAD_REQUEST);
}

#[test]
fn body_of_the_wrong_shape_is_a_bad_request() {
    let body = serde_json::to_value(&WrongShapeBody {
        template_id: "t".to_string(),
        inputs: 3,
    })
    .unwrap();
    assert_eq!(
        call(Method::Post, RUNS_PATH, body).unwrap().status,
        BAD_REQUEST
    );
}

#[test]
fn unknown_project_is_not_found() {
    let body = body_of(GHOST_PROJECT, TEMPLATE_DEV);
    assert_eq!(
        call(Method::Post, RUNS_PATH, body).unwrap().status,
        NOT_FOUND
    );
}

#[test]
fn unknown_run_is_not_found() {
    let read = call(Method::Get, "/api/runs/nope", Value::Null).unwrap();
    assert_eq!(read.status, NOT_FOUND);
    let cancel = call(Method::Post, "/api/runs/nope/cancel", Value::Null).unwrap();
    assert_eq!(cancel.status, NOT_FOUND);
}

#[test]
fn conflicting_transition_maps_to_409() {
    let error = super::super::conflict("run r is not paused".to_string());
    assert_eq!(error_reply(&error).status, CONFLICT);
}

#[test]
fn other_paths_and_verbs_are_not_handled() {
    assert!(call(Method::Get, "/api/flows", Value::Null).is_none());
    assert!(call(Method::Post, "/api/runs/r/stages/s/explode", Value::Null).is_none());
}

#[test]
fn a_relative_workspace_is_a_bad_request() {
    let body = serde_json::to_value(&NativeRunRequest {
        project_id: PROJECT_ID.to_string(),
        template_id: TEMPLATE_DEV.to_string(),
        workspace: Some(std::path::PathBuf::from(RELATIVE_WORKSPACE)),
        ..NativeRunRequest::default()
    })
    .unwrap();
    let reply = call(Method::Post, RUNS_PATH, body).unwrap();
    assert_eq!(reply.status, BAD_REQUEST);
}

/// The vault choice of a launch survives the trip through the request body.
#[test]
fn the_vault_choice_of_a_launch_reaches_the_run_request() {
    let own_vault = std::env::temp_dir().join(OWN_VAULT);
    let launch = StartNativeRun {
        project_id: PROJECT_ID.to_string(),
        template_id: TEMPLATE_DEV.to_string(),
        vault: Some(own_vault.clone()),
        no_vault: true,
        ..StartNativeRun::default()
    };

    let request = parse_request(&serde_json::to_value(&launch).unwrap()).unwrap();

    assert_eq!(request.vault, Some(own_vault));
    assert!(request.no_vault);
}

/// A body that says nothing about the vault leaves the choice to the daemon.
#[test]
fn a_body_without_a_vault_choice_leaves_it_to_the_daemon() {
    let request = parse_request(&body_of(PROJECT_ID, TEMPLATE_DEV)).unwrap();

    assert_eq!(request.vault, None);
    assert!(!request.no_vault);
}

/// A headless launch stays headless through the request body and the persisted run request.
#[test]
fn a_headless_launch_survives_parsing_and_persistence() {
    let launch = StartNativeRun {
        project_id: PROJECT_ID.to_string(),
        template_id: TEMPLATE_DEV.to_string(),
        headless: true,
        ..StartNativeRun::default()
    };

    let request = parse_request(&serde_json::to_value(&launch).unwrap()).unwrap();
    let persisted: StartNativeRun =
        serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();

    assert!(request.headless);
    assert!(persisted.headless);
    assert!(
        !parse_request(&body_of(PROJECT_ID, TEMPLATE_DEV))
            .unwrap()
            .headless
    );
}
