//! `/api/memory` and `/api/rag` over HTTP, on a temporary store and a mini project.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

const VAULT_ENV: &str = "AIDP_VAULT";

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

struct Booted {
    handle: ServeHandle,
    port: u16,
    _state: &'static TempWorkspace,
}

fn boot(vault: &Path, projects: Vec<Value>, read_only: bool) -> Booted {
    let state: &'static TempWorkspace = Box::leak(Box::new(TempWorkspace::new("gw-mem-state")));
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
        read_only,
        allowed_origins: vec![],
    };
    Booted {
        handle: serve_on(listener, config).unwrap(),
        port,
        _state: state,
    }
}

fn project_entry(name: &str, path: &Path) -> Value {
    json!({ "id": name, "name": name, "path": path.to_string_lossy().replace('\\', "/") })
}

fn mini_project() -> TempWorkspace {
    let project = TempWorkspace::new("gw-mem-project");
    let docs = project.path().join("Docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(
        docs.join("movement.md"),
        "# Movement\n\nThe dash has a cooldown of two seconds.\n",
    )
    .unwrap();
    project
}

fn lesson() -> Value {
    json!({
        "kind": "lesson", "title": "Rigidbody velocity in FixedUpdate",
        "text": "Assign the velocity in FixedUpdate, never in Update.",
        "scope": { "route": "code", "paths": ["Assets/Scripts/Player/**"], "tags": ["physics"] },
        "refs": [ { "type": "issue", "id": "GH-214" } ], "confidence": 0.8
    })
}

#[test]
fn memory_life_cycle_over_http_with_events() {
    let vault = TempWorkspace::new("gw-mem-vault");
    let project = mini_project();
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let booted = boot(
        vault.path(),
        vec![project_entry("game", project.path())],
        false,
    );
    let port = booted.port;
    let events = booted.handle.app().bus.subscribe();

    let (status, created) = send("POST", port, "/api/memory", lesson());
    assert_eq!(status, 201, "{created}");
    let memory = &created["memory"];
    assert_eq!(memory["id"], "M-1");
    assert_eq!(memory["project"], "game");
    assert_eq!(memory["status"], "proposed");
    assert_eq!(memory["scope"]["route"], "code");
    assert_eq!(memory["refs"][0]["type"], "issue");
    assert!(memory["createdAt"].is_string());
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap()["type"],
        "memory.proposed"
    );

    let (_, second) = send(
        "POST",
        port,
        "/api/memory",
        json!({ "kind": "fact", "title": "Other", "text": "Something else entirely about audio." }),
    );
    assert_eq!(second["memory"]["id"], "M-2");

    let (_, listed) = get(port, "/api/memory?status=proposed&q=velocity");
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let (bad, _) = get(port, "/api/memory?status=bogus");
    assert_eq!(bad, 400);

    let (status, patched) = send(
        "PATCH",
        port,
        "/api/memory/M-1",
        json!({ "title": "FixedUpdate velocity" }),
    );
    assert_eq!(status, 200, "{patched}");
    assert_eq!(patched["title"], "FixedUpdate velocity");

    let (_, approved) = send("POST", port, "/api/memory/M-1/approve", json!({}));
    assert_eq!(approved["status"], "active");
    let (status, merged) = send(
        "POST",
        port,
        "/api/memory/M-2/merge",
        json!({ "into": "M-1" }),
    );
    assert_eq!(status, 200, "{merged}");
    let (_, archived) = send("POST", port, "/api/memory/M-1/archive", json!({}));
    assert_eq!(archived["status"], "archived");
    let (status, _) = send("POST", port, "/api/memory/M-1/approve", json!({}));
    assert_eq!(status, 409);
    let (status, _) = get(port, "/api/memory/M-99");
    assert_eq!(status, 404);
    let (status, _) = send("POST", port, "/api/memory/M-1/merge", json!({}));
    assert_eq!(status, 400);

    let changed: Vec<Value> =
        std::iter::from_fn(|| events.recv_timeout(Duration::from_millis(300)).ok())
            .filter(|event| event["type"] == "memory.changed")
            .collect();
    assert!(changed
        .iter()
        .any(|e| e["data"]["id"] == "M-1" && e["data"]["status"] == "archived"));
    booted.handle.stop();
}

#[test]
fn reindex_then_search_returns_cited_excerpts() {
    let vault = TempWorkspace::new("gw-mem-vault");
    let project = mini_project();
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let booted = boot(
        vault.path(),
        vec![project_entry("game", project.path())],
        false,
    );
    let port = booted.port;
    let events = booted.handle.app().bus.subscribe();

    let (status, body) = send("POST", port, "/api/rag/reindex", json!({ "source": "doc" }));
    assert_eq!(status, 202, "{body}");
    let done = loop {
        let event = events
            .recv_timeout(Duration::from_secs(30))
            .expect("rag.reindexed");
        if event["type"] == "rag.reindexed" {
            break event["data"].clone();
        }
    };
    assert_eq!(done["ok"], true);
    assert_eq!(done["indexed"], 1);

    let (status, found) = get(
        port,
        "/api/rag/search?q=dash%20cooldown&source=doc&budget=500",
    );
    assert_eq!(status, 200, "{found}");
    let hit = &found["hits"][0];
    assert_eq!(hit["source"], "doc");
    assert_eq!(hit["path"], "Docs/movement.md");
    assert!(hit["citation"]
        .as_str()
        .unwrap()
        .starts_with("Docs/movement.md:L"));
    assert!(hit["lines"].as_str().unwrap().starts_with("L"));
    assert!(hit["score"].is_number());
    assert!(hit["text"].as_str().unwrap().contains("cooldown"));

    let (missing, _) = get(port, "/api/rag/search");
    assert_eq!(missing, 400);
    booted.handle.stop();
}

#[test]
fn project_rule_and_read_only_mode() {
    let vault = TempWorkspace::new("gw-mem-vault");
    let first = mini_project();
    let second = mini_project();
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let booted = boot(
        vault.path(),
        vec![
            project_entry("first", first.path()),
            project_entry("second", second.path()),
        ],
        false,
    );
    let (ambiguous, _) = get(booted.port, "/api/memory");
    let (by_name, _) = get(booted.port, "/api/memory?project=second");
    assert_eq!((ambiguous, by_name), (400, 200));
    booted.handle.stop();

    let read_only = boot(
        vault.path(),
        vec![project_entry("game", first.path())],
        true,
    );
    let (read, _) = get(read_only.port, "/api/memory");
    let (write, _) = send("POST", read_only.port, "/api/memory", lesson());
    let (reindex, _) = send("POST", read_only.port, "/api/rag/reindex", json!({}));
    assert_eq!(read, 200);
    assert!(write >= 400 && reindex >= 400, "{write} {reindex}");
    read_only.handle.stop();
}

fn knowledge_note(domain: &str, title: &str) -> String {
    format!(
        "---\ntype: note\ndomain: {domain}\ntags: []\nagents: []\nstatus: active\nimportance: 0.5\nupdated: 2026-11-11\nsource: test\n---\n# {title}\n\nUne regle pour {title}.\n"
    )
}

#[test]
fn search_filters_knowledge_by_domain_and_agent_type() {
    let vault = TempWorkspace::new("gw-mem-vault");
    for (domain, title) in [("unity", "saut"), ("pixel-art", "contour")] {
        let dir = vault.path().join("Knowledge/_global").join(domain);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("note.md"), knowledge_note(domain, title)).unwrap();
    }
    let project = mini_project();
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let booted = boot(
        vault.path(),
        vec![project_entry("game", project.path())],
        false,
    );
    let port = booted.port;
    let events = booted.handle.app().bus.subscribe();
    let (status, body) = send(
        "POST",
        port,
        "/api/rag/reindex",
        json!({ "source": "knowledge" }),
    );
    assert_eq!(status, 202, "{body}");
    loop {
        let event = events
            .recv_timeout(Duration::from_secs(30))
            .expect("rag.reindexed");
        if event["type"] == "rag.reindexed" && event["data"]["trigger"] == "api" {
            break;
        }
    }

    let paths = |query: &str| -> Vec<String> {
        let (status, found) = get(port, &format!("/api/rag/search?source=knowledge&{query}"));
        assert_eq!(status, 200, "{found}");
        found["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hit| hit["path"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(paths("q=regle").len(), 2);
    assert_eq!(
        paths("q=regle&domain=unity"),
        ["Knowledge/_global/unity/note.md"]
    );
    assert_eq!(
        paths("q=regle&agentType=pixel-artist"),
        ["Knowledge/_global/pixel-art/note.md"]
    );
    assert_eq!(
        paths("q=regle&domain=unity&agentType=pixel-artist"),
        ["Knowledge/_global/unity/note.md"]
    );
    assert_eq!(paths("q=regle&lexical=true").len(), 2);
    let (status, _) = get(port, "/api/rag/search?q=regle&domain=cooking");
    assert_eq!(status, 400);
    booted.handle.stop();
}
