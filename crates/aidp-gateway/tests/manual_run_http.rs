//! Manual launches over HTTP: the `flow` override and `force` of `POST /api/tickets/:key/start`,
//! `POST /api/runs/start` refusals, and the inputs `GET /api/catalog/templates` serves.
//! Nothing here launches a session: every start is refused or stopped before one exists.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::{EnvGuard, TempWorkspace};
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::store::TicketStore;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::Path;

const VAULT_ENV: &str = "AIDP_VAULT";
const SKILL_ENV: &str = "AIDP_SKILL_ROOT";
const GAME_CODE_FLOW: &str = include_str!("fixtures/game_flows/game-code.yaml");
const TEMPLATE: &str = "version: 5
id: manual-demo
kind: pipeline
description: One stage, one required input.
inputs:
  topic: { required: true, kind: string, description: What to write about }
  mood: { required: false, kind: string, default: calm, description: Tone }
stages:
  - name: draft
    description: Write the draft.
    agent: analyzer
    outputs: [draft.md]
";
const KEY: &str = "LOC-1";
const SERVICE: &str = "game";

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

struct Env {
    vault: TempWorkspace,
    project: TempWorkspace,
    _skill: TempWorkspace,
    _env: EnvGuard,
}

fn env(tag: &str) -> Env {
    let vault = TempWorkspace::new(&format!("{tag}-vault"));
    let project = TempWorkspace::new(&format!("{tag}-project"));
    let skill = TempWorkspace::new(&format!("{tag}-skill"));
    fs::create_dir_all(skill.child("templates/flows")).unwrap();
    fs::write(skill.child("templates/manual-demo.yaml"), TEMPLATE).unwrap();
    fs::write(
        skill.child("templates/flows/game-code.yaml"),
        GAME_CODE_FLOW,
    )
    .unwrap();
    let guard = EnvGuard::with(&[
        (VAULT_ENV, Some(vault.path().to_str().unwrap())),
        (SKILL_ENV, Some(skill.path().to_str().unwrap())),
    ]);
    Env {
        vault,
        project,
        _skill: skill,
        _env: guard,
    }
}

fn boot(env: &Env, projects: Vec<Value>) -> (ServeHandle, u16) {
    let state: &'static TempWorkspace = Box::leak(Box::new(TempWorkspace::new("gw-manual-state")));
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
        vault_root: env.vault.path().to_path_buf(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    (serve_on(listener, config).unwrap(), port)
}

fn seed_ticket(vault: &Path, status: Status, reason: Option<&str>) {
    let now = chrono::Utc::now();
    let autopilot = Autopilot {
        blocked_reason: reason.map(str::to_string),
        ..Autopilot::default()
    };
    let mut ticket = Ticket {
        key: KEY.to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "sample".to_string(),
        status,
        priority: Priority::Medium,
        priority_rank: 0,
        service: SERVICE.to_string(),
        template: Some("manual-demo".to_string()),
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
        autopilot,
        tags: Vec::new(),
    };
    TicketStore::open(vault)
        .unwrap()
        .save(&mut ticket, "", &serde_yaml::Mapping::new())
        .unwrap();
}

fn stored(vault: &Path) -> Ticket {
    TicketStore::open(vault).unwrap().get(KEY).unwrap().0
}

fn write_dispatcher_config(vault: &Path, yaml: &str) {
    TicketStore::open(vault).unwrap();
    fs::write(vault.join("TODO").join("config.yaml"), yaml).unwrap();
}

fn start_path() -> String {
    format!("/api/tickets/{KEY}/start")
}

#[test]
fn an_unknown_flow_override_is_a_400_and_changes_nothing() {
    let env = env("gw-manual-flow-unknown");
    seed_ticket(env.vault.path(), Status::Todo, None);
    let (handle, port) = boot(&env, vec![]);

    let (status, body) = post(port, &start_path(), json!({ "flow": "no-such-flow" }));

    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("no-such-flow"));
    let ticket = stored(env.vault.path());
    assert_eq!(ticket.status, Status::Todo);
    assert_eq!(ticket.flow, None);
    handle.stop();
}

#[test]
fn a_known_flow_override_replaces_the_routed_flow_and_a_template_alone_clears_it() {
    let env = env("gw-manual-flow-known");
    seed_ticket(env.vault.path(), Status::Todo, None);
    write_dispatcher_config(
        env.vault.path(),
        "dispatcher:\n  paused: true\n  slots: 5\n",
    );
    let (handle, port) = boot(&env, vec![]);

    // Paused without force: refused with 423 after the overrides were persisted on the note.
    let (status, body) = post(port, &start_path(), json!({ "flow": "game-code" }));
    assert_eq!(status, 423, "{body}");
    assert_eq!(stored(env.vault.path()).flow.as_deref(), Some("game-code"));

    let (status, _) = post(port, &start_path(), json!({ "template": "manual-demo" }));
    assert_eq!(status, 423);
    let ticket = stored(env.vault.path());
    assert_eq!(ticket.flow, None, "a template alone means pipeline only");
    assert_eq!(ticket.template.as_deref(), Some("manual-demo"));
    handle.stop();
}

#[test]
fn force_starts_a_blocked_ticket_past_a_paused_dispatcher() {
    let env = env("gw-manual-force");
    seed_ticket(env.vault.path(), Status::Blocked, Some("not-ready"));
    // No free slot: a forced start waives the pause but is then declined by the slot ceiling,
    // which proves the pause no longer answers 423 and the blocked ticket was re-armed.
    write_dispatcher_config(
        env.vault.path(),
        "dispatcher:\n  paused: true\n  slots: 0\n",
    );
    let project = env.project.path().to_path_buf();
    let (handle, port) = boot(&env, vec![project_entry("p1", SERVICE, &project)]);

    let (status, body) = post(port, &start_path(), json!({ "force": true }));

    assert_eq!(status, 409, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("no-slot"),
        "forced past the pause: {body}"
    );
    assert_eq!(stored(env.vault.path()).status, Status::Queued);
    handle.stop();
}

#[test]
fn a_blocked_ticket_without_force_is_not_waived_past_a_paused_dispatcher() {
    let env = env("gw-manual-noforce");
    seed_ticket(env.vault.path(), Status::Blocked, Some("needs-refine"));
    write_dispatcher_config(
        env.vault.path(),
        "dispatcher:\n  paused: true\n  slots: 0\n",
    );
    let project = env.project.path().to_path_buf();
    let (handle, port) = boot(&env, vec![project_entry("p1", SERVICE, &project)]);

    let (status, body) = post(port, &start_path(), json!({}));

    assert_eq!(status, 423, "{body}");
    handle.stop();
}

#[test]
fn runs_start_refuses_bad_requests_before_launching_anything() {
    let env = env("gw-manual-runs");
    let project = env.project.path().to_path_buf();
    let other = TempWorkspace::new("gw-manual-runs-other");
    let (handle, port) = boot(
        &env,
        vec![
            project_entry("p1", "game", &project),
            project_entry("p2", "tools", other.path()),
        ],
    );
    let path = "/api/runs/start";

    let (status, body) = post(port, path, json!({ "template": "manual-demo" }));
    assert_eq!(status, 400, "several projects and none named: {body}");

    let (status, _) = post(
        port,
        path,
        json!({ "project": "nope", "template": "manual-demo" }),
    );
    assert_eq!(status, 404);

    let named = |extra: Value| {
        let mut body = json!({ "project": "game" });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        post(port, path, body)
    };
    let (status, body) = named(json!({}));
    assert_eq!(
        (status, body["error"].as_str().unwrap()),
        (400, "template is required")
    );
    let (status, body) = named(json!({ "template": "nope" }));
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("unknown template"));
    let (status, body) = named(json!({ "template": "manual-demo" }));
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("'topic'"));

    handle.stop();
}

#[test]
fn the_template_catalogue_serves_the_declared_inputs() {
    let env = env("gw-manual-catalog");
    let (handle, port) = boot(&env, vec![]);

    let (status, body) = get(port, "/api/catalog/templates");

    assert_eq!(status, 200, "{body}");
    let template = body
        .as_array()
        .unwrap()
        .iter()
        .find(|template| template["id"] == "manual-demo")
        .expect("manual-demo is listed");
    assert_eq!(template["inputCount"], 2);
    assert_eq!(template["inputs"]["topic"]["required"], true);
    assert_eq!(
        template["inputs"]["topic"]["description"],
        "What to write about"
    );
    assert!(template["inputs"]["topic"]["default"].is_null());
    assert_eq!(template["inputs"]["mood"]["required"], false);
    assert_eq!(template["inputs"]["mood"]["default"], "calm");
    assert_eq!(template["inputs"]["mood"]["kind"], "string");
    handle.stop();
}
