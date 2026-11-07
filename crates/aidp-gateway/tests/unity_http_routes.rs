//! `GET /api/unity/status` and `POST /api/unity/lock/release` over HTTP.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::Path;

const VAULT_ENV: &str = "AIDP_VAULT";
const STATUS_PATH: &str = "/api/unity/status";
const RELEASE_PATH: &str = "/api/unity/lock/release";
const STATUS_OK: u16 = 200;
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_NOT_FOUND: u16 = 404;

fn url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

fn reply(result: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
    match result {
        Ok(resp) => (resp.status(), resp.into_json().unwrap_or(Value::Null)),
        Err(ureq::Error::Status(code, resp)) => (code, resp.into_json().unwrap_or(Value::Null)),
        Err(error) => panic!("request failed: {error}"),
    }
}

fn get(port: u16, path: &str) -> (u16, Value) {
    reply(ureq::get(&url(port, path)).call())
}

fn post(port: u16, path: &str, body: Value) -> (u16, Value) {
    reply(ureq::post(&url(port, path)).send_json(body))
}

fn project_entry(id: &str, name: &str, path: &Path) -> Value {
    json!({ "id": id, "name": name, "path": path.to_string_lossy().replace('\\', "/") })
}

struct Booted {
    handle: ServeHandle,
    port: u16,
    _state: &'static TempWorkspace,
}

fn boot(vault: &Path, projects: Vec<Value>) -> Booted {
    let state: &'static TempWorkspace = Box::leak(Box::new(TempWorkspace::new("gw-unity-state")));
    fs::write(state.path().join(".defaults-seeded"), "").unwrap();
    fs::write(
        state.path().join("projects.json"),
        serde_json::to_vec(&projects).unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir: state.path().to_path_buf(),
        vault_root: vault.to_path_buf(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    Booted {
        handle: serve_on(listener, config).unwrap(),
        port,
        _state: state,
    }
}

fn write_result(project: &Path, kind: &str, name: &str, body: &Value) {
    let dir = project.join(".aidp").join(kind).join("results");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(name), body.to_string()).unwrap();
}

#[test]
fn status_reports_a_free_lock_without_editor_and_the_latest_results() {
    let vault = TempWorkspace::new("gw-unity-vault");
    let project = TempWorkspace::new("gw-unity-project");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    write_result(
        project.path(),
        "compile",
        "c1.json",
        &json!({ "id": "c1", "ok": true, "errors": [], "warnings": 0 }),
    );
    write_result(
        project.path(),
        "playtest",
        "p1.json",
        &json!({ "id": "issue-1-ca1", "passed": true }),
    );
    let booted = boot(
        vault.path(),
        vec![project_entry("p1", "game", project.path())],
    );

    let (status, body) = get(booted.port, STATUS_PATH);

    assert_eq!(status, STATUS_OK, "{body}");
    assert_eq!(body["editor"]["running"], false);
    assert!(body["editor"]["pid"].is_null());
    assert_eq!(
        body["editor"]["projectPath"],
        project.path().to_string_lossy().replace('\\', "/").as_str()
    );
    assert_eq!(body["lock"]["held"], false);
    assert!(body["lock"]["holder"].is_null());
    assert_eq!(body["lastCompile"]["id"], "c1");
    assert_eq!(body["lastPlaytest"]["id"], "issue-1-ca1");
    assert_eq!(body["lastPlaytest"]["passed"], true);
    assert!(body["lastPlaytest"]["finishedAt"].is_string());
    booted.handle.stop();
}

#[test]
fn several_projects_need_the_project_query() {
    let vault = TempWorkspace::new("gw-unity-vault-many");
    let first = TempWorkspace::new("gw-unity-first");
    let second = TempWorkspace::new("gw-unity-second");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let booted = boot(
        vault.path(),
        vec![
            project_entry("p1", "first", first.path()),
            project_entry("p2", "second", second.path()),
        ],
    );

    let (ambiguous, _) = get(booted.port, STATUS_PATH);
    let (by_name, body) = get(booted.port, &format!("{STATUS_PATH}?project=second"));
    let (unknown, _) = get(booted.port, &format!("{STATUS_PATH}?project=nope"));

    assert_eq!(ambiguous, STATUS_BAD_REQUEST);
    assert_eq!(by_name, STATUS_OK);
    assert_eq!(body["project"], "p2");
    assert_eq!(unknown, STATUS_NOT_FOUND);
    booted.handle.stop();
}

#[test]
fn release_of_a_free_lock_answers_released_false() {
    let vault = TempWorkspace::new("gw-unity-vault-release");
    let project = TempWorkspace::new("gw-unity-project-release");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let booted = boot(
        vault.path(),
        vec![project_entry("p1", "game", project.path())],
    );

    let (status, body) = post(booted.port, RELEASE_PATH, json!({}));

    assert_eq!(status, STATUS_OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["released"], false);
    assert_eq!(body["resource"], "unity-editor:p1");
    assert!(body["holder"].is_null());
    booted.handle.stop();
}
