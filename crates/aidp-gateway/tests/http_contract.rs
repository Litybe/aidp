//! HTTP contract tests: 9 scenarios against a really-started daemon.
//! These tests verify the HTTP API behavior by starting a ServeHandle on port 0,
//! seeding fixtures, and making actual HTTP requests through ureq.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────
// URL constants and builders
// ─────────────────────────────────────────────────────────────────────────────

const API_BASE: &str = "/api";
const NON_API_PATH: &str = "/nope";

const GATEWAY_PREFIX: &str = "/gateway";
const SESSIONS_PREFIX: &str = "/sessions";
const HOOKS_PREFIX: &str = "/hooks";
const USAGE_PREFIX: &str = "/usage";
const DISPATCHER_PREFIX: &str = "/dispatcher";
const HEALTH_PREFIX: &str = "/health";
const PRESETS_PREFIX: &str = "/presets";
const TICKETS_PREFIX: &str = "/tickets";
const AUTOMATIONS_PREFIX: &str = "/automations";
const NOT_FOUND_SEGMENT: &str = "/nope";

const STATUS_SEGMENT: &str = "status";
const PLAN_SEGMENT: &str = "plan";
const CLEANUP_SEGMENT: &str = "cleanup";
const START_SEGMENT: &str = "start";
const TEST_SEGMENT: &str = "test";
const EXPORT_SEGMENT: &str = "export.md";
const GROUP_BY_PARAM: &str = "groupBy";
const WORKTREES_PREFIX: &str = "/worktrees";
const WORKTREE_SEGMENT: &str = "worktree";
const PRUNE_SEGMENT: &str = "prune";
const ORPHANED_SESSION_ID: &str = "s-orphaned";
const ORPHANED_STATUS: &str = "orphaned";
const PRUNE_PROJECT_ID: &str = "proj-a";
const VAULT_ENV: &str = "AIDP_VAULT";
const DISABLED_VAULT: &str = "off";
const NO_VAULT_ERROR: &str = "no vault configured";
const UNKNOWN_TICKET_KEY: &str = "NOPE-1";

const UNKNOWN_SESSION_ID: &str = "does-not-exist";
const UNKNOWN_PROJECT_ID: &str = "nosuchproject";
const UNKNOWN_PRESET_ID: &str = "nosuchpreset";
const UNKNOWN_AUTOMATION_ID: &str = "nosuchautomation";
const TEST_HOOK_ID: &str = "test-id";

/// URL of the gateway status endpoint.
fn gateway_status_url(port: u16) -> String {
    make_url(
        port,
        &format!("{API_BASE}{GATEWAY_PREFIX}/{STATUS_SEGMENT}"),
    )
}

/// URL of the sessions collection endpoint.
fn sessions_url(port: u16) -> String {
    make_url(port, &format!("{API_BASE}{SESSIONS_PREFIX}"))
}

/// URL of a single session's export endpoint.
fn session_export_url(port: u16, session_id: &str) -> String {
    make_url(
        port,
        &format!("{API_BASE}{SESSIONS_PREFIX}/{session_id}/{EXPORT_SEGMENT}"),
    )
}

/// URL of a single session resource.
fn session_url(port: u16, session_id: &str) -> String {
    make_url(port, &format!("{API_BASE}{SESSIONS_PREFIX}/{session_id}"))
}

/// URL of a hook endpoint by automation id.
fn hook_url(port: u16, automation_id: &str) -> String {
    make_url(port, &format!("{API_BASE}{HOOKS_PREFIX}/{automation_id}"))
}

/// URL of the usage endpoint filtered by `groupBy`.
fn usage_group_by_url(port: u16, value: &str) -> String {
    make_url(
        port,
        &format!("{API_BASE}{USAGE_PREFIX}?{GROUP_BY_PARAM}={value}"),
    )
}

/// URL of the dispatcher plan endpoint.
fn dispatcher_plan_url(port: u16) -> String {
    make_url(
        port,
        &format!("{API_BASE}{DISPATCHER_PREFIX}/{PLAN_SEGMENT}"),
    )
}

/// URL of the health cleanup endpoint.
fn health_cleanup_url(port: u16) -> String {
    make_url(
        port,
        &format!("{API_BASE}{HEALTH_PREFIX}/{CLEANUP_SEGMENT}"),
    )
}

/// URL to start a saved preset by id.
fn preset_start_url(port: u16, preset_id: &str) -> String {
    make_url(
        port,
        &format!("{API_BASE}{PRESETS_PREFIX}/{preset_id}/{START_SEGMENT}"),
    )
}

/// URL of the worktrees listing.
fn worktrees_url(port: u16) -> String {
    make_url(port, &format!("{API_BASE}{WORKTREES_PREFIX}"))
}

/// URL of the worktree prune endpoint.
fn worktrees_prune_url(port: u16) -> String {
    make_url(
        port,
        &format!("{API_BASE}{WORKTREES_PREFIX}/{PRUNE_SEGMENT}"),
    )
}

/// URL of one ticket's worktree.
fn ticket_worktree_url(port: u16, key: &str) -> String {
    make_url(
        port,
        &format!("{API_BASE}{TICKETS_PREFIX}/{key}/{WORKTREE_SEGMENT}"),
    )
}

/// URL to test a saved automation by id.
fn automation_test_url(port: u16, automation_id: &str) -> String {
    make_url(
        port,
        &format!("{API_BASE}{AUTOMATIONS_PREFIX}/{automation_id}/{TEST_SEGMENT}"),
    )
}

/// URL of an unknown `/api`-prefixed route.
fn api_not_found_url(port: u16) -> String {
    make_url(port, &format!("{API_BASE}{NOT_FOUND_SEGMENT}"))
}

/// URL of an unknown route with no `/api` prefix.
fn root_not_found_url(port: u16) -> String {
    make_url(port, NON_API_PATH)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build a complete SessionRecord with all required fields for testing.
fn complete_session_record(id: &str, project_id: &str, cost_usd: f64, status: &str) -> Value {
    json!({
        "id": id,
        "projectId": project_id,
        "projectName": format!("Project {}", project_id),
        "cwd": "/tmp/test",
        "engine": "v7",
        "prompt": "test prompt",
        "permissionMode": "open",
        "status": status,
        "startedAt": "2024-01-01T00:00:00.000Z",
        "origin": {
            "kind": "api"
        },
        "costUsd": cost_usd,
    })
}

/// Boot and stop harness: creates temp workspace, serves on port 0, returns
/// (state_dir, handle, port).
fn boot_server() -> (PathBuf, ServeHandle, u16) {
    boot_server_with_vault(PathBuf::new())
}

fn boot_server_with_vault(vault_root: PathBuf) -> (PathBuf, ServeHandle, u16) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-http")));
    let state_dir = workspace.path().to_path_buf();

    // Create .defaults-seeded to prevent AutomationEngine from seeding defaults.
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();
    assert_ne!(port, 0, "port must be non-zero");

    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root,
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };

    let handle = serve_on(listener, config).expect("serve_on failed");
    assert_eq!(handle.port(), port, "returned port must match bound port");

    (state_dir, handle, port)
}

fn make_url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{}{}", port, path)
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 1: Lifecycle — start on port 0, check status, stop in <2s
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_1_lifecycle() {
    let (state_dir, handle, port) = boot_server();

    // Assert port is non-zero and bound.
    assert_ne!(port, 0);

    // GET /api/gateway/status returns 200.
    {
        let resp = ureq::get(&gateway_status_url(port))
            .call()
            .expect("GET /api/gateway/status failed");
        assert_eq!(resp.status(), 200);
        let _: Value = resp.into_json().expect("invalid JSON response");
    } // Ensure response is dropped before stop()

    // .defaults-seeded must exist (we created it), but automations.json must NOT.
    assert!(state_dir.join(".defaults-seeded").exists());
    assert!(!state_dir.join("automations.json").exists());

    // Stop and assert <2s elapsed.
    let t = Instant::now();
    handle.stop();
    let elapsed = t.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "stop() took {:?}, expected <2s",
        elapsed
    );

    // Verify port can be rebound (with a small delay for OS socket cleanup on Windows).
    std::thread::sleep(Duration::from_millis(100));
    TcpListener::bind(("127.0.0.1", port)).expect("port not rebindable after stop");
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 2: Read-only mode
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_2_read_only_mode() {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-http-ro")));
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();

    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: true, // ← read-only mode
        allowed_origins: vec![],
    };

    let handle = serve_on(listener, config).expect("serve_on failed");

    // POST /api/sessions with empty body should return 403.
    let resp = ureq::post(&sessions_url(port))
        .send_json(json!({}))
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 403);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(
        body["error"].as_str(),
        Some("read-only mode: set AIDP_READ_ONLY=0 to allow writes")
    );

    // POST /api/hooks/:id should NOT return 403 (it's exempt).
    let resp = ureq::post(&hook_url(port, TEST_HOOK_ID))
        .set("X-Aidp-Token", "hook-secret")
        .send_json(json!({}))
        .unwrap_or_else(|e| e.into_response().unwrap());
    // The /hooks route may return 404 (automation not found) or 200, but NOT 403.
    assert_ne!(resp.status(), 403);

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 3: Invalid groupBy parameter
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_3_invalid_groupby() {
    let (_, handle, port) = boot_server();

    // GET /api/usage?groupBy=bogus should return 400.
    let resp = ureq::get(&usage_group_by_url(port, "bogus"))
        .call()
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 400);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(
        body["error"].as_str(),
        Some("groupBy must be one of day|project|template|origin|ticket|model|stage|route")
    );

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 4: Not found routes (both /api and without /api prefix)
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_4_not_found() {
    let (_, handle, port) = boot_server();

    // GET /api/nope should return 404.
    let resp = ureq::get(&api_not_found_url(port))
        .call()
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 404);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(body["error"].as_str(), Some("not found"));

    // GET /nope (without /api prefix) should also return 404 with same body.
    let resp = ureq::get(&root_not_found_url(port))
        .call()
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 404);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(body["error"].as_str(), Some("not found"));

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 13: every error envelope carries `code` — a 404 and a 401
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_13_error_envelope_carries_code() {
    let (_, handle, port) = boot_server();

    // GET /api/sessions/<unknown> is a 404 with code "not_found".
    let resp = ureq::get(&session_url(port, UNKNOWN_SESSION_ID))
        .call()
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 404);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(body["error"].as_str(), Some("session not found"));
    assert_eq!(body["code"].as_str(), Some("not_found"));

    // POST /api/hooks/:id with a wrong bearer token is a 401 with code "unauthorized".
    let resp = ureq::post(&hook_url(port, TEST_HOOK_ID))
        .set("X-Aidp-Token", "wrong-token")
        .send_json(json!({}))
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 401);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(body["code"].as_str(), Some("unauthorized"));

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 5: OPTIONS CORS preflight
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_5_options_cors() {
    let (_, handle, port) = boot_server();

    // OPTIONS /api/sessions with no Authorization should return 204.
    let resp = ureq::request("OPTIONS", &sessions_url(port))
        .call()
        .expect("OPTIONS request failed");
    assert_eq!(resp.status(), 204);

    // Body must be empty.
    let text = resp.into_string().expect("failed to read body");
    assert!(text.is_empty(), "expected empty body, got: {}", text);

    // Check headers by re-issuing the request.
    let resp = ureq::request("OPTIONS", &sessions_url(port))
        .call()
        .expect("OPTIONS request failed");

    let allow_methods = resp.header("Access-Control-Allow-Methods");
    let allow_origin = resp.header("Access-Control-Allow-Origin");

    assert_eq!(
        allow_methods,
        Some("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
        "missing or wrong Access-Control-Allow-Methods header"
    );
    // LOC-26: no Origin header sent -> no Access-Control-Allow-Origin (no wildcard).
    assert_eq!(
        allow_origin, None,
        "no Origin sent, so no Access-Control-Allow-Origin expected"
    );

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 6: Usage aggregation with groupBy=project
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_6_usage_aggregation() {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-http-usage")));
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");

    // Create sessions directory.
    let sessions_dir = state_dir.join("sessions");
    fs::create_dir_all(&sessions_dir).expect("failed to create sessions dir");

    // Seed three complete session fixtures with known costs and two project IDs.
    let sess1 = complete_session_record("sess-1", "proj-a", 1.5, "done");
    let sess2 = complete_session_record("sess-2", "proj-b", 2.5, "done");
    let sess3 = complete_session_record("sess-3", "proj-a", 3.0, "done");

    fs::write(
        sessions_dir.join("sess-1.json"),
        serde_json::to_string(&sess1).unwrap(),
    )
    .expect("failed to write sess-1.json");
    fs::write(
        sessions_dir.join("sess-2.json"),
        serde_json::to_string(&sess2).unwrap(),
    )
    .expect("failed to write sess-2.json");
    fs::write(
        sessions_dir.join("sess-3.json"),
        serde_json::to_string(&sess3).unwrap(),
    )
    .expect("failed to write sess-3.json");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();

    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };

    let handle = serve_on(listener, config).expect("serve_on failed");

    // GET /api/usage?groupBy=project should return 200 with buckets.len() == 2.
    let resp = ureq::get(&usage_group_by_url(port, "project"))
        .call()
        .expect("GET /api/usage failed");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.into_json().expect("invalid JSON response");

    let buckets = body["buckets"]
        .as_array()
        .expect("buckets must be an array");
    assert_eq!(
        buckets.len(),
        2,
        "expected 2 buckets, got {}",
        buckets.len()
    );

    // Check that totals.estimatedCostUsd equals sum of three costs (1.5 + 2.5 + 3.0).
    let total_cost = body["totals"]["estimatedCostUsd"]
        .as_f64()
        .expect("estimatedCostUsd must be a number");
    let expected_cost = 1.5 + 2.5 + 3.0;
    assert!(
        (total_cost - expected_cost).abs() < 0.01,
        "expected {}, got {}",
        expected_cost,
        total_cost
    );

    // Write a FOURTH fixture AFTER boot — it should not appear in the results.
    let sess4 = complete_session_record("sess-4", "proj-c", 0.5, "done");
    fs::write(
        sessions_dir.join("sess-4.json"),
        serde_json::to_string(&sess4).unwrap(),
    )
    .expect("failed to write sess-4.json");

    // Query again — should still have 2 buckets (not 3), and cost should not include sess-4.
    let resp = ureq::get(&usage_group_by_url(port, "project"))
        .call()
        .expect("GET /api/usage failed");
    let body: Value = resp.into_json().expect("invalid JSON response");
    let buckets = body["buckets"]
        .as_array()
        .expect("buckets must be an array");
    assert_eq!(
        buckets.len(),
        2,
        "expected 2 buckets after late write, got {}",
        buckets.len()
    );

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 7: GET /api/dispatcher/plan with no-vault mode
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_7_dispatcher_plan_no_vault() {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-http-plan")));
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();

    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: PathBuf::new(), // ← empty vault root = no-vault mode
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };

    let handle = serve_on(listener, config).expect("serve_on failed");

    // GET /api/dispatcher/plan should return 200.
    let resp = ureq::get(&dispatcher_plan_url(port))
        .call()
        .expect("GET /api/dispatcher/plan failed");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.into_json().expect("invalid JSON response");

    // Body should be exactly {"mode":"no-vault","launches":[],"skipped":[]}.
    assert_eq!(body["mode"].as_str(), Some("no-vault"));
    assert_eq!(body["launches"].as_array().map(|a| a.len()), Some(0));
    assert_eq!(body["skipped"].as_array().map(|a| a.len()), Some(0));

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 8: Export endpoint with seeded sessions and events
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_8_export() {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-http-export")));
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");

    let sessions_dir = state_dir.join("sessions");
    fs::create_dir_all(&sessions_dir).expect("failed to create sessions dir");

    // Seed sess-export.json (complete record).
    let sess = complete_session_record("sess-export", "proj-a", 1.0, "done");
    fs::write(
        sessions_dir.join("sess-export.json"),
        serde_json::to_string(&sess).unwrap(),
    )
    .expect("failed to write sess-export.json");

    // Seed sess-export.events.jsonl with a dummy event.
    let event = json!({"type": "text", "text": "hello"});
    fs::write(
        sessions_dir.join("sess-export.events.jsonl"),
        serde_json::to_string(&event).unwrap() + "\n",
    )
    .expect("failed to write sess-export.events.jsonl");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();

    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };

    let handle = serve_on(listener, config).expect("serve_on failed");

    // GET /api/sessions/sess-export/export.md should return 200 with text/markdown.
    let resp = ureq::get(&session_export_url(port, "sess-export"))
        .call()
        .expect("GET /api/sessions/sess-export/export.md failed");
    assert_eq!(resp.status(), 200);

    // Check Content-Type header.
    let ct = resp.header("Content-Type");
    assert_eq!(
        ct,
        Some("text/markdown; charset=utf-8"),
        "missing or wrong Content-Type header"
    );

    // Re-request to check body is markdown.
    let resp = ureq::get(&session_export_url(port, "sess-export"))
        .call()
        .expect("GET /api/sessions/sess-export/export.md failed");
    let body = resp.into_string().expect("failed to read body");
    assert!(!body.is_empty(), "export.md body must not be empty");

    // GET /api/sessions/sess-absente/export.md should return 404.
    let resp = ureq::get(&session_export_url(port, "sess-absente"))
        .call()
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 404);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(body["error"].as_str(), Some("session not found"));

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 9: Cleanup — purge orphaned sessions
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_9_cleanup() {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-http-cleanup")));
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");

    let sessions_dir = state_dir.join("sessions");
    fs::create_dir_all(&sessions_dir).expect("failed to create sessions dir");

    // Seed four complete fixtures: three orphaned (s-c, s-a, s-b), one done.
    let orphaned_c = complete_session_record("s-c", "proj-a", 1.0, "orphaned");
    let orphaned_a = complete_session_record("s-a", "proj-a", 1.0, "orphaned");
    let orphaned_b = complete_session_record("s-b", "proj-a", 1.0, "orphaned");
    let done = complete_session_record("s-done", "proj-a", 1.0, "done");

    fs::write(
        sessions_dir.join("s-c.json"),
        serde_json::to_string(&orphaned_c).unwrap(),
    )
    .expect("failed to write s-c.json");
    fs::write(
        sessions_dir.join("s-a.json"),
        serde_json::to_string(&orphaned_a).unwrap(),
    )
    .expect("failed to write s-a.json");
    fs::write(
        sessions_dir.join("s-b.json"),
        serde_json::to_string(&orphaned_b).unwrap(),
    )
    .expect("failed to write s-b.json");
    fs::write(
        sessions_dir.join("s-done.json"),
        serde_json::to_string(&done).unwrap(),
    )
    .expect("failed to write s-done.json");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();

    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };

    let handle = serve_on(listener, config).expect("serve_on failed");

    // POST /api/health/cleanup should return 200 with {"ok":true,"count":3,"purged":["s-a","s-b","s-c"]}.
    let resp = ureq::post(&health_cleanup_url(port))
        .send_json(json!({}))
        .expect("POST /api/health/cleanup failed");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.into_json().expect("invalid JSON response");

    assert_eq!(body["ok"].as_bool(), Some(true));
    assert_eq!(body["count"].as_u64(), Some(3));

    let purged = body["purged"].as_array().expect("purged must be an array");
    assert_eq!(purged.len(), 3);

    // The purged list is sorted, so check for ["s-a", "s-b", "s-c"].
    let purged_strs: Vec<String> = purged
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert_eq!(purged_strs, vec!["s-a", "s-b", "s-c"]);

    // Verify the three orphaned files are gone from disk, but s-done.json remains.
    assert!(
        !sessions_dir.join("s-a.json").exists(),
        "s-a.json should be deleted"
    );
    assert!(
        !sessions_dir.join("s-b.json").exists(),
        "s-b.json should be deleted"
    );
    assert!(
        !sessions_dir.join("s-c.json").exists(),
        "s-c.json should be deleted"
    );
    assert!(
        sessions_dir.join("s-done.json").exists(),
        "s-done.json should still exist"
    );

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 10: POST /api/sessions against an unknown project answers 404
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_10_start_session_unknown_project_is_not_found() {
    let (_, handle, port) = boot_server();

    let resp = ureq::post(&sessions_url(port))
        .send_json(json!({ "projectId": UNKNOWN_PROJECT_ID }))
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 404);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert!(body.get("error").is_some(), "body must carry an error key");

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 11: POST /api/presets/:id/start against an unknown preset answers 404
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_11_start_preset_unknown_preset_is_not_found() {
    let (_, handle, port) = boot_server();

    let resp = ureq::post(&preset_start_url(port, UNKNOWN_PRESET_ID))
        .send_json(json!({}))
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 404);

    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 14: the automation endpoint keeps its forced status code on
// an untyped anyhow error (not the api_err() generic 400 fallback)
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_14_automation_not_found_keeps_forced_status() {
    let (_, handle, port) = boot_server();

    let resp = ureq::post(&automation_test_url(port, UNKNOWN_AUTOMATION_ID))
        .send_json(json!({}))
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(
        resp.status(),
        404,
        "an unknown automation id must stay 404, not fall through to api_err's 400"
    );

    handle.stop();
}

/// The worktree read routes answer 503 without a vault.
#[test]
fn scenario_15_worktree_routes_without_vault_are_unavailable() {
    let _env = EnvGuard::with(&[(VAULT_ENV, Some(DISABLED_VAULT))]);
    let (_, handle, port) = boot_server();

    for url in [
        worktrees_url(port),
        ticket_worktree_url(port, UNKNOWN_TICKET_KEY),
    ] {
        let resp = ureq::get(&url)
            .call()
            .unwrap_or_else(|e| e.into_response().unwrap());
        assert_eq!(
            resp.status(),
            503,
            "{url} must be unavailable without a vault"
        );
        let body: Value = resp.into_json().expect("invalid JSON response");
        assert_eq!(body["error"].as_str(), Some(NO_VAULT_ERROR));
    }

    handle.stop();
}

/// The worktree read routes list worktrees, serialize their shape and answer 404 for an unknown ticket.
#[test]
fn scenario_16_worktree_routes_shape_and_not_found() {
    let vault = Box::leak(Box::new(TempWorkspace::new("gw-http-worktrees-vault")));
    let _env = EnvGuard::with(&[(VAULT_ENV, Some(vault.path().to_str().expect("utf8 path")))]);
    let (_, handle, port) = boot_server_with_vault(vault.path().to_path_buf());

    let resp = ureq::get(&worktrees_url(port)).call().expect("list failed");
    assert_eq!(resp.status(), 200);
    let listed: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(listed, json!([]), "no project registered: empty list");

    let resp = ureq::get(&ticket_worktree_url(port, UNKNOWN_TICKET_KEY))
        .call()
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 404, "an unknown ticket must be 404");

    let created: Value = ureq::post(&make_url(port, &format!("{API_BASE}{TICKETS_PREFIX}")))
        .send_json(json!({
            "service": "svc",
            "summary": "worktree shape",
            "allowUnknownService": true,
        }))
        .expect("create failed")
        .into_json()
        .expect("invalid JSON response");
    let key = created["key"].as_str().expect("created key").to_string();

    let resp = ureq::get(&ticket_worktree_url(port, &key))
        .call()
        .expect("worktree failed");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(
        body["branch"].as_str(),
        Some(format!("feat/{key}").as_str())
    );
    assert_eq!(body["exists"], json!(false));
    assert_eq!(body["modified"], json!(0));
    assert!(body["path"]
        .as_str()
        .is_some_and(|p| p.ends_with(".worktree")));

    handle.stop();
}

/// The worktree prune route answers 503 without a vault, like the other worktree routes.
#[test]
fn worktree_prune_without_vault_is_unavailable() {
    let _env = EnvGuard::with(&[(VAULT_ENV, Some(DISABLED_VAULT))]);
    let (_, handle, port) = boot_server();

    let resp = ureq::post(&worktrees_prune_url(port))
        .send_json(json!({}))
        .unwrap_or_else(|e| e.into_response().unwrap());
    assert_eq!(resp.status(), 503);
    let body: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(body["error"].as_str(), Some(NO_VAULT_ERROR));

    handle.stop();
}

/// The worktree prune route only sweeps worktrees and leaves orphaned sessions on disk.
#[test]
fn worktree_prune_does_not_purge_orphaned_sessions() {
    let vault = Box::leak(Box::new(TempWorkspace::new("gw-http-prune-vault")));
    let _env = EnvGuard::with(&[(VAULT_ENV, Some(vault.path().to_str().expect("utf8 path")))]);
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-http-prune-state")));
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");
    let sessions_dir = state_dir.join("sessions");
    fs::create_dir_all(&sessions_dir).expect("failed to create sessions dir");
    let orphaned_file = sessions_dir.join(format!("{ORPHANED_SESSION_ID}.json"));
    let orphaned =
        complete_session_record(ORPHANED_SESSION_ID, PRUNE_PROJECT_ID, 1.0, ORPHANED_STATUS);
    fs::write(&orphaned_file, serde_json::to_string(&orphaned).unwrap())
        .expect("failed to write orphaned session");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();
    let config = HttpConfig {
        state_dir,
        vault_root: vault.path().to_path_buf(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    let handle = serve_on(listener, config).expect("serve_on failed");

    let resp = ureq::post(&worktrees_prune_url(port))
        .send_json(json!({}))
        .expect("prune failed");
    assert_eq!(resp.status(), 200);
    assert!(orphaned_file.exists(), "prune must not purge sessions");

    handle.stop();
}
