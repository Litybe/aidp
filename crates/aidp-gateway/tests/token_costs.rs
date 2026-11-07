//! `GET /api/usage?groupBy=ticket|model|route`, `GET /api/tickets/:key/cost` and
//! `GET /api/cost/anomalies` over sessions seeded in the state directory.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::TempWorkspace;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

const STATUS_OK: u16 = 200;
const STATUS_NOT_FOUND: u16 = 404;

fn seed_session(sessions_dir: &Path, id: &str, ticket: Option<&str>, input_tokens: u64) {
    let mut origin = json!({ "kind": "flow", "label": "implement" });
    if let Some(ticket) = ticket {
        origin["ref"] = json!(ticket);
    }
    let record = json!({
        "id": id,
        "projectId": "p",
        "projectName": "P",
        "cwd": "/tmp/test",
        "engine": "v7",
        "templateId": "dev-unity",
        "prompt": "x",
        "permissionMode": "open",
        "status": "done",
        "startedAt": "2026-11-06T10:00:00.000Z",
        "model": "claude-sonnet-5",
        "origin": origin,
        "usage": { "input": input_tokens, "output": 0, "cacheRead": 0, "cacheCreate": 0 },
    });
    fs::write(
        sessions_dir.join(format!("{id}.json")),
        serde_json::to_string(&record).unwrap(),
    )
    .unwrap();
}

fn boot(tickets: &[(&str, u64)]) -> (ServeHandle, u16) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-token-costs")));
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").unwrap();
    let sessions_dir = state_dir.join("sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    for (index, (ticket, tokens)) in tickets.iter().enumerate() {
        seed_session(&sessions_dir, &format!("s{index}"), Some(ticket), *tokens);
    }
    seed_session(&sessions_dir, "orphan", None, 5);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir,
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    (serve_on(listener, config).unwrap(), port)
}

fn get(port: u16, path: &str) -> (u16, Value) {
    let response = ureq::get(&format!("http://127.0.0.1:{port}{path}"))
        .call()
        .unwrap_or_else(|error| error.into_response().unwrap());
    let status = response.status();
    (status, response.into_json().unwrap_or(Value::Null))
}

/// Sessions group by the ticket key carried in `origin.ref`.
#[test]
fn usage_group_by_ticket_buckets_flow_sessions_by_origin_ref() {
    let (handle, port) = boot(&[("GH-1", 100), ("GH-1", 50), ("GH-2", 10)]);
    let (status, body) = get(port, "/api/usage?groupBy=ticket");
    assert_eq!(status, STATUS_OK);
    let buckets = body["buckets"].as_array().unwrap();
    assert_eq!(buckets[0]["key"], "GH-1");
    assert_eq!(buckets[0]["totalTokens"], 150);
    assert_eq!(buckets[0]["sessions"], 2);
    assert_eq!(buckets[1]["key"], "GH-2");
    assert_eq!(buckets[2]["key"], "unknown");
    handle.stop();
}

/// `groupBy=model` and `groupBy=route` answer with the same envelope.
#[test]
fn usage_group_by_model_and_route_are_served() {
    let (handle, port) = boot(&[("GH-1", 100)]);
    let (status, by_model) = get(port, "/api/usage?groupBy=model");
    assert_eq!(status, STATUS_OK);
    assert_eq!(by_model["buckets"][0]["key"], "claude-sonnet-5");
    let (status, by_route) = get(port, "/api/usage?groupBy=route");
    assert_eq!(status, STATUS_OK);
    assert_eq!(by_route["groupBy"], "route");
    assert_eq!(by_route["totals"]["totalTokens"], 105);
    handle.stop();
}

/// The cost of one ticket carries its totals and the breakdowns.
#[test]
fn ticket_cost_reports_totals_and_breakdowns() {
    let (handle, port) = boot(&[("GH-1", 100), ("GH-1", 50), ("GH-2", 10)]);
    let (status, body) = get(port, "/api/tickets/GH-1/cost");
    assert_eq!(status, STATUS_OK);
    assert_eq!(body["key"], "GH-1");
    assert_eq!(body["totals"]["totalTokens"], 150);
    assert_eq!(body["totals"]["input"], 150);
    assert_eq!(body["byStep"][0]["key"], "implement");
    assert_eq!(body["byFlow"][0]["key"], "dev-unity");
    assert_eq!(body["route"], "code");
    handle.stop();
}

/// A ticket nobody worked on is a 404.
#[test]
fn ticket_cost_of_an_unknown_ticket_is_not_found() {
    let (handle, port) = boot(&[("GH-1", 100)]);
    let (status, _) = get(port, "/api/tickets/GH-99/cost");
    assert_eq!(status, STATUS_NOT_FOUND);
    handle.stop();
}

/// One ticket far above the others is flagged; two tickets are not enough to say.
#[test]
fn cost_anomalies_flag_the_outlier_and_need_three_tickets() {
    let (handle, port) = boot(&[("GH-1", 100)]);
    let (_, body) = get(port, "/api/cost/anomalies");
    assert_eq!(body["status"], "insufficient_data");
    handle.stop();

    let (handle, port) = boot(&[
        ("GH-1", 100),
        ("GH-2", 110),
        ("GH-3", 105),
        ("GH-4", 95),
        ("GH-5", 9000),
    ]);
    let (status, body) = get(port, "/api/cost/anomalies");
    assert_eq!(status, STATUS_OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["anyFlagged"], true);
    let flagged: Vec<&Value> = body["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|point| point["flagged"] == true)
        .collect();
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0]["key"], "GH-5");
    handle.stop();
}
