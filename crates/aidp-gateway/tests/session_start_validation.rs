//! LOC-24: `POST /api/sessions` rejects a tainted `model`, `effort` or
//! `permissionMode` with 400 before anything is queued or spawned.
//!
//! Every request in this file is invalid on purpose: a fully valid request
//! would spawn a real `claude` process on the developer machine.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_gateway::models::enums::WorkspaceKind;
use aidp_gateway::models::WorkspaceRequest;
use aidp_gateway::sessions::StartRequest;
use aidp_testkit::TempWorkspace;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

const ENGINE_PROMPT: &str = "prompt";
const PROMPT_TEXT: &str = "x";
const UNKNOWN_TICKET: &str = "LOC-9999";
const UNKNOWN_TRANSCRIPT_ID: &str = "00000000-0000-4000-8000-000000000000";

/// Serializes a prompt-engine start request for `project_id` with the given workspace.
fn workspace_body(project_id: &str, workspace: WorkspaceRequest) -> Value {
    let request = StartRequest {
        project_id: project_id.to_string(),
        engine: Some(ENGINE_PROMPT.to_string()),
        prompt: Some(PROMPT_TEXT.to_string()),
        workspace: Some(workspace),
        ..Default::default()
    };
    serde_json::to_value(&request).expect("failed to serialize StartRequest")
}

fn boot_server() -> (PathBuf, ServeHandle, u16) {
    boot_server_with_vault(&PathBuf::new())
}

fn boot_server_with_vault(vault: &Path) -> (PathBuf, ServeHandle, u16) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-validate")));
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();

    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: vault.to_path_buf(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    let handle = serve_on(listener, config).expect("serve_on failed");
    (state_dir, handle, port)
}

fn make_url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{}{}", port, path)
}

/// POST a JSON body and return (status, parsed body), whatever the status.
fn post(port: u16, path: &str, body: Value) -> (u16, Value) {
    let resp = ureq::post(&make_url(port, path))
        .send_json(body)
        .unwrap_or_else(|e| e.into_response().expect("transport error"));
    let status = resp.status();
    (status, resp.into_json().expect("invalid JSON response"))
}

/// Register a fresh project directory; returns (project id, project path).
fn register_project(state_dir: &std::path::Path, port: u16) -> (String, PathBuf) {
    let repo = state_dir.join("repo");
    fs::create_dir_all(&repo).expect("failed to create project dir");
    let (status, body) = post(
        port,
        "/api/projects",
        json!({ "path": repo.to_string_lossy() }),
    );
    assert_eq!(status, 200, "project registration failed: {body}");
    let id = body["id"].as_str().expect("project id missing").to_string();
    (id, repo)
}

/// Send a start request that must be rejected; returns the `error` string.
fn start_rejected(port: u16, body: Value) -> String {
    let (status, resp) = post(port, "/api/sessions", body);
    assert_eq!(status, 400, "expected 400, got {status}: {resp}");
    resp["error"]
        .as_str()
        .unwrap_or_else(|| panic!("no error string in {resp}"))
        .to_string()
}

fn assert_no_sessions(port: u16, project_id: &str) {
    let resp = ureq::get(&make_url(
        port,
        &format!("/api/sessions?projectId={project_id}"),
    ))
    .call()
    .expect("session list failed");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.into_json().expect("invalid JSON response");
    let items = body
        .as_array()
        .or_else(|| body["items"].as_array())
        .or_else(|| body["sessions"].as_array())
        .unwrap_or_else(|| panic!("unexpected session list shape: {body}"));
    assert!(items.is_empty(), "no session may be recorded: {body}");
}

#[test]
fn post_sessions_rejects_model_with_cmd_metacharacters() {
    let (state_dir, handle, port) = boot_server();
    let (id, repo) = register_project(&state_dir, port);

    let error = start_rejected(
        port,
        json!({
            "projectId": id,
            "engine": "prompt",
            "prompt": "x",
            "model": "sonnet\" & echo AIDP_INJECTED_MARKER > marker.txt & rem \"",
        }),
    );
    assert!(error.contains("model"), "error must name model: {error}");
    assert!(
        !repo.join("marker.txt").exists(),
        "marker.txt created in project dir"
    );
    assert!(
        !PathBuf::from("marker.txt").exists(),
        "marker.txt created in cwd"
    );
    assert_no_sessions(port, &id);

    handle.stop();
}

#[test]
fn post_sessions_rejects_unknown_effort() {
    let (state_dir, handle, port) = boot_server();
    let (id, _) = register_project(&state_dir, port);

    let error = start_rejected(
        port,
        json!({ "projectId": id, "engine": "prompt", "prompt": "x", "effort": "ultra" }),
    );
    for v in ["low", "medium", "high", "xhigh", "max"] {
        assert!(error.contains(v), "error must list {v}: {error}");
    }
    assert_no_sessions(port, &id);

    handle.stop();
}

#[test]
fn post_sessions_rejects_unknown_permission_mode() {
    let (state_dir, handle, port) = boot_server();
    let (id, _) = register_project(&state_dir, port);

    let error = start_rejected(
        port,
        json!({
            "projectId": id,
            "engine": "prompt",
            "prompt": "x",
            "permissionMode": "bypassPermissions & whoami",
        }),
    );
    for v in ["default", "acceptEdits", "bypassPermissions", "plan"] {
        assert!(error.contains(v), "error must list {v}: {error}");
    }
    assert_no_sessions(port, &id);

    handle.stop();
}

#[test]
fn post_sessions_rejects_model_with_whitespace_and_percent() {
    let (state_dir, handle, port) = boot_server();
    let (id, _) = register_project(&state_dir, port);

    for model in ["sonnet opus", "%COMSPEC%"] {
        let error = start_rejected(
            port,
            json!({ "projectId": id, "engine": "prompt", "prompt": "x", "model": model }),
        );
        assert!(
            error.contains("model"),
            "error must name model for {model:?}: {error}"
        );
    }
    assert_no_sessions(port, &id);

    handle.stop();
}

/// Send a start request and return (status, error string), whatever the status.
fn start_status(port: u16, body: Value) -> (u16, String) {
    let (status, resp) = post(port, "/api/sessions", body);
    let error = resp["error"].as_str().unwrap_or_default().to_string();
    (status, error)
}

#[test]
fn post_sessions_refuses_a_path_workspace_outside_the_allowed_roots() {
    let (state_dir, handle, port) = boot_server();
    let (id, _) = register_project(&state_dir, port);
    let outside = TempWorkspace::new("gw-outside-root");

    let (status, error) = start_status(
        port,
        workspace_body(
            &id,
            WorkspaceRequest {
                kind: WorkspaceKind::Path,
                ticket: None,
                name: None,
                path: Some(outside.path().to_string_lossy().into_owned()),
            },
        ),
    );

    assert_eq!(status, 403, "{error}");
    assert!(error.contains("allowed roots"), "{error}");
    assert_no_sessions(port, &id);
    handle.stop();
}

#[test]
fn post_sessions_answers_conflict_when_the_ticket_has_no_worktree() {
    let vault = TempWorkspace::new("gw-ticket-vault");
    let _vault_env = aidp_testkit::EnvGuard::set("AIDP_VAULT", vault.path().to_str().unwrap());
    let (state_dir, handle, port) = boot_server_with_vault(vault.path());
    let (id, _) = register_project(&state_dir, port);

    let (status, error) = start_status(
        port,
        workspace_body(
            &id,
            WorkspaceRequest {
                kind: WorkspaceKind::Ticket,
                ticket: Some(UNKNOWN_TICKET.to_string()),
                name: None,
                path: None,
            },
        ),
    );

    assert_eq!(status, 409, "{error}");
    assert_no_sessions(port, &id);
    handle.stop();
}

#[test]
fn post_sessions_answers_not_found_for_an_unknown_external_session() {
    let (state_dir, handle, port) = boot_server();
    let (id, _) = register_project(&state_dir, port);

    let (status, error) = start_status(
        port,
        serde_json::to_value(StartRequest {
            project_id: id.clone(),
            engine: Some(ENGINE_PROMPT.to_string()),
            prompt: Some(PROMPT_TEXT.to_string()),
            resume_from: Some(UNKNOWN_TRANSCRIPT_ID.to_string()),
            external: Some(true),
            ..Default::default()
        })
        .expect("failed to serialize StartRequest"),
    );

    assert_eq!(status, 404, "{error}");
    assert_no_sessions(port, &id);
    handle.stop();
}

#[test]
fn claude_sessions_route_lists_for_a_known_project_and_404s_for_an_unknown_one() {
    let (state_dir, handle, port) = boot_server();
    let (id, _) = register_project(&state_dir, port);

    let listed = ureq::get(&make_url(
        port,
        &format!("/api/projects/{id}/claude-sessions"),
    ))
    .call()
    .expect("claude-sessions listing failed");
    assert_eq!(listed.status(), 200);
    let body: Value = listed.into_json().expect("invalid JSON response");
    assert!(body.is_array(), "expected an array: {body}");

    let missing = ureq::get(&make_url(port, "/api/projects/nope/claude-sessions"))
        .call()
        .expect_err("unknown project must fail");
    assert_eq!(missing.into_response().expect("response").status(), 404);
    handle.stop();
}
