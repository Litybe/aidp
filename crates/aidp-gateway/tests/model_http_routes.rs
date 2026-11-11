//! `/api/memory/model` and `/api/embed` over HTTP, with a fake model loader and a temporary gateway directory.

use aidp_gateway::http::{serve_on_with_environment, HttpConfig, ServeHandle};
use aidp_gateway::model_service::ModelLoader;
use aidp_gateway::models::runtime_environment::RuntimeEnvironment;
use aidp_memory::models::constants::embedding::DEFAULT_MODEL_NAME;
use aidp_memory::Embedder;
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const VAULT_ENV: &str = "AIDP_VAULT";
const FAKE_DIM: usize = 3;
const FAKE_BYTES: u64 = 4096;
const WAIT: Duration = Duration::from_secs(30);
const MODEL_PATH: &str = "/api/memory/model";

struct FakeEmbedder;

impl Embedder for FakeEmbedder {
    fn model_id(&self) -> &str {
        DEFAULT_MODEL_NAME
    }

    fn dim(&self) -> usize {
        FAKE_DIM
    }

    fn embed(&self, texts: &[&str]) -> aidp_memory::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| vec![text.len() as f32, 1.0, 0.0])
            .collect())
    }
}

struct FakeLoader {
    installed: AtomicBool,
}

impl ModelLoader for FakeLoader {
    fn installed(&self, _: &Path) -> bool {
        self.installed.load(Ordering::SeqCst)
    }

    fn weights_bytes(&self, _: &Path) -> Option<u64> {
        Some(FAKE_BYTES)
    }

    fn load(&self, _: &Path) -> Result<Arc<dyn Embedder>, String> {
        Ok(Arc::new(FakeEmbedder))
    }
}

struct Booted {
    handle: ServeHandle,
    port: u16,
    state: &'static TempWorkspace,
}

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

fn send(method: &str, port: u16, path: &str, body: Value) -> (u16, Value) {
    reply(ureq::request(method, &url(port, path)).send_json(body))
}

fn boot(vault: &Path, projects: Vec<Value>, installed: bool, read_only: bool) -> Booted {
    boot_with(vault, projects, installed, read_only, false)
}

fn boot_with(
    vault: &Path,
    projects: Vec<Value>,
    installed: bool,
    read_only: bool,
    autostart: bool,
) -> Booted {
    let state: &'static TempWorkspace = Box::leak(Box::new(TempWorkspace::new("gw-model-state")));
    fs::write(state.path().join(".defaults-seeded"), "").unwrap();
    fs::write(
        state.path().join("projects.json"),
        serde_json::to_vec(&projects).unwrap(),
    )
    .unwrap();
    if !autostart {
        fs::create_dir_all(state.path().join("memory")).unwrap();
        fs::write(
            state.path().join("memory/model.json"),
            r#"{ "autostart": false }"#,
        )
        .unwrap();
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir: state.path().to_path_buf(),
        vault_root: vault.to_path_buf(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only,
        allowed_origins: vec![],
    };
    let environment = RuntimeEnvironment {
        model_loader: Some(Arc::new(FakeLoader {
            installed: AtomicBool::new(installed),
        })),
        ..Default::default()
    };
    Booted {
        handle: serve_on_with_environment(listener, config, environment).unwrap(),
        port,
        state,
    }
}

fn wait_state(port: u16, state: &str) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        let (_, body) = get(port, MODEL_PATH);
        if body["state"] == state {
            return body;
        }
        assert!(Instant::now() < deadline, "never {state}: {body}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_flat_error(body: &Value, code: &str) {
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["code", "error"], "{body}");
    assert_eq!(body["code"], code, "{body}");
    assert!(body["error"].as_str().is_some_and(|text| !text.is_empty()));
}

fn mini_project() -> TempWorkspace {
    let project = TempWorkspace::new("gw-model-project");
    let docs = project.path().join("Docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(
        docs.join("movement.md"),
        "# Movement\n\nThe dash has a cooldown of two seconds.\n",
    )
    .unwrap();
    project
}

#[test]
fn status_of_a_missing_model_has_the_documented_shape() {
    let vault = TempWorkspace::new("gw-model-vault");
    let booted = boot(vault.path(), vec![], false, false);

    let (status, body) = get(booted.port, MODEL_PATH);

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["model"], DEFAULT_MODEL_NAME);
    assert_eq!(body["installed"], false);
    assert_eq!(body["state"], "stopped");
    assert_eq!(body["autostart"], false);
    for null_field in ["dim", "ramBytes", "loadedAt", "error"] {
        assert!(body[null_field].is_null(), "{null_field}: {body}");
    }
    assert_eq!(
        body["chunks"],
        json!({ "total": 0, "embedded": 0, "stale": 0 })
    );
    assert!(body["path"].as_str().unwrap().contains(DEFAULT_MODEL_NAME));
}

#[test]
fn start_without_the_model_is_a_model_missing_conflict() {
    let vault = TempWorkspace::new("gw-model-vault");
    let booted = boot(vault.path(), vec![], false, false);

    let (status, body) = send("POST", booted.port, "/api/memory/model/start", json!({}));

    assert_eq!(status, 409);
    assert_flat_error(&body, "model_missing");
}

#[test]
fn start_then_embed_then_stop_over_http_with_events() {
    let vault = TempWorkspace::new("gw-model-vault");
    let booted = boot(vault.path(), vec![], true, false);
    let port = booted.port;
    let events = booted.handle.app().bus.subscribe();

    let (status, started) = send("POST", port, "/api/memory/model/start", json!({}));
    assert_eq!(status, 202, "{started}");
    assert!(["loading", "running"].contains(&started["state"].as_str().unwrap()));
    let running = wait_state(port, "running");
    assert_eq!(running["dim"], FAKE_DIM);
    assert_eq!(running["ramBytes"], FAKE_BYTES);
    assert!(running["loadedAt"].is_string());

    let (status, embedded) = send(
        "POST",
        port,
        "/api/embed",
        json!({ "texts": ["coyote time", "saut"] }),
    );
    assert_eq!(status, 200, "{embedded}");
    assert_eq!(embedded["model"], DEFAULT_MODEL_NAME);
    assert_eq!(embedded["dim"], FAKE_DIM);
    assert_eq!(
        embedded["vectors"],
        json!([[11.0, 1.0, 0.0], [4.0, 1.0, 0.0]])
    );

    let (status, stopped) = send("POST", port, "/api/memory/model/stop", json!({}));
    assert_eq!(status, 200, "{stopped}");
    assert_eq!(stopped["state"], "stopped");
    let seen: Vec<String> = events
        .try_iter()
        .filter(|event| event["type"] == "memory.model")
        .map(|event| event["data"]["state"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(seen, ["loading", "running", "stopped"]);
}

#[test]
fn a_stopped_model_refuses_embedding_and_pending_work() {
    let vault = TempWorkspace::new("gw-model-vault");
    let booted = boot(vault.path(), vec![], true, false);

    let (status, body) = send(
        "POST",
        booted.port,
        "/api/embed",
        json!({ "texts": ["coyote"] }),
    );
    assert_eq!(status, 409);
    assert_flat_error(&body, "model_stopped");

    let (status, body) = send(
        "POST",
        booted.port,
        "/api/memory/model/embed-pending",
        json!({}),
    );
    assert_eq!(status, 409);
    assert_flat_error(&body, "model_stopped");
}

#[test]
fn an_embed_batch_over_the_cap_is_refused_before_the_model_is_asked() {
    let vault = TempWorkspace::new("gw-model-vault");
    let booted = boot(vault.path(), vec![], true, false);
    let texts: Vec<String> = (0..257).map(|number| number.to_string()).collect();

    let (status, body) = send("POST", booted.port, "/api/embed", json!({ "texts": texts }));

    assert_eq!(status, 400);
    assert_flat_error(&body, "batch_too_large");

    let (status, body) = send("POST", booted.port, "/api/embed", json!({ "text": "x" }));
    assert_eq!(status, 400);
    assert_flat_error(&body, "bad_request");
}

#[test]
fn autostart_is_saved_and_a_bad_body_is_a_flat_bad_request() {
    let vault = TempWorkspace::new("gw-model-vault");
    let booted = boot_with(vault.path(), vec![], true, false, true);
    let port = booted.port;
    assert_eq!(get(port, MODEL_PATH).1["autostart"], true);
    wait_state(port, "running");

    let (status, body) = send("PATCH", port, MODEL_PATH, json!({ "autostart": false }));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["autostart"], false);
    assert_eq!(get(port, MODEL_PATH).1["autostart"], false);
    let saved = fs::read_to_string(booted.state.path().join("memory/model.json")).unwrap();
    assert!(saved.contains("false"), "{saved}");

    let (status, body) = send("PATCH", port, MODEL_PATH, json!({ "autostart": "yes" }));
    assert_eq!(status, 400);
    assert_flat_error(&body, "bad_request");
}

#[test]
fn read_only_mode_still_embeds_but_cannot_start_or_stop() {
    let vault = TempWorkspace::new("gw-model-vault");
    let booted = boot(vault.path(), vec![], true, true);

    let (status, _) = send("POST", booted.port, "/api/memory/model/start", json!({}));
    assert_eq!(status, 403);
    let (status, body) = send("POST", booted.port, "/api/embed", json!({ "texts": ["a"] }));
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["code"], "model_stopped");
}

#[test]
fn embed_pending_gives_the_indexed_chunks_their_vectors() {
    let vault = TempWorkspace::new("gw-model-vault");
    let project = mini_project();
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let game = json!({
        "id": "game", "name": "game",
        "path": project.path().to_string_lossy().replace('\\', "/")
    });
    let booted = boot(vault.path(), vec![game], true, false);
    let port = booted.port;
    let events = booted.handle.app().bus.subscribe();

    let (status, _) = send("POST", port, "/api/rag/reindex", json!({ "source": "doc" }));
    assert_eq!(status, 202);
    wait_reindexed(&events, "api");
    let before = get(port, MODEL_PATH).1["chunks"].clone();
    assert!(before["total"].as_u64().unwrap() > 0, "{before}");
    assert_eq!(before["embedded"], 0);

    send("POST", port, "/api/memory/model/start", json!({}));
    wait_state(port, "running");
    let (status, queued) = send("POST", port, "/api/memory/model/embed-pending", json!({}));
    assert_eq!(status, 202, "{queued}");
    assert_eq!(queued, json!({ "queued": true }));
    let done = wait_reindexed(&events, "embed-pending");

    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(done["embedded"], before["total"]);
    let after = get(port, MODEL_PATH).1["chunks"].clone();
    assert_eq!(after["embedded"], before["total"], "{after}");
    assert_eq!(after["stale"], 0);
}

fn wait_reindexed(events: &std::sync::mpsc::Receiver<Value>, trigger: &str) -> Value {
    loop {
        let event = events.recv_timeout(WAIT).expect("rag.reindexed");
        if event["type"] == "rag.reindexed" && event["data"]["trigger"] == trigger {
            return event["data"].clone();
        }
    }
}
