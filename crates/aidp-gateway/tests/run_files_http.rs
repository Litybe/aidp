//! `/api/runs/:id/files` and the ticket `flowSummary` on a real daemon: fixture run directory,
//! listing, PNG type, traversal refusal, unknown run, and a ticket without a flow.

use aidp_core::models::run_state::RunState;
use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_gateway::models::native_run_start::StartNativeRun;
use aidp_gateway::native_runs::recovery;
use aidp_gateway::native_runs::state::RunStore;
use aidp_testkit::{EnvGuard, TempWorkspace};
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::store::TicketStore;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

const RUN_ID: &str = "run-files-1";
const VAULT_ENV: &str = "AIDP_VAULT";
const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const TICKET_KEY: &str = "LOC-9";

struct Fixture {
    _workspace: TempWorkspace,
    _vault: TempWorkspace,
    _env: EnvGuard,
    handle: ServeHandle,
    port: u16,
}

fn ticket() -> Ticket {
    let now = chrono::Utc::now();
    Ticket {
        key: TICKET_KEY.to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "sample".to_string(),
        status: Status::Todo,
        priority: Priority::Medium,
        priority_rank: 0,
        service: "svc".to_string(),
        template: None,
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
        autopilot: Autopilot::default(),
        tags: Vec::new(),
    }
}

fn boot() -> Fixture {
    let workspace = TempWorkspace::new("gw-run-files");
    let vault = TempWorkspace::new("gw-run-files-vault");
    let env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    fs::create_dir_all(vault.child("Pipelines/svc")).unwrap();
    let store = TicketStore::open(vault.path()).unwrap();
    store
        .save(&mut ticket(), "", &serde_yaml::Mapping::new())
        .unwrap();

    let state_dir = workspace.child("state");
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(state_dir.join(".defaults-seeded"), "").unwrap();
    let run_dir = workspace.child("run");
    fs::create_dir_all(run_dir.join("plan")).unwrap();
    fs::create_dir_all(run_dir.join("implement/shots")).unwrap();
    let state = RunState {
        run_id: RUN_ID.to_string(),
        run_dir: run_dir.clone(),
        ..RunState::default()
    };
    RunStore::create(state, &StartNativeRun::default(), workspace.child("repo")).unwrap();
    recovery::index_add(&recovery::index_path(&state_dir), RUN_ID, &run_dir).unwrap();
    fs::write(run_dir.join("plan/plan.md"), "# plan").unwrap();
    fs::write(run_dir.join("implement/shots/a.png"), PNG_MAGIC).unwrap();
    fs::write(workspace.child("secret.txt"), "outside").unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir,
        vault_root: vault.path().to_path_buf(),
        hook_token: String::new(),
        ui_token: String::new(),
        read_only: false,
        allowed_origins: vec![],
    };
    let handle = serve_on(listener, config).unwrap();
    Fixture {
        _workspace: workspace,
        _vault: vault,
        _env: env,
        handle,
        port,
    }
}

fn get(port: u16, path: &str) -> ureq::Response {
    match ureq::get(&format!("http://127.0.0.1:{port}{path}")).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(error) => panic!("GET {path}: {error}"),
    }
}

fn raw(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

#[test]
fn run_files_are_listed_and_served_confined() {
    let fx = boot();
    let list: Value = get(fx.port, &format!("/api/runs/{RUN_ID}/files"))
        .into_json()
        .unwrap();
    let paths: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"plan/plan.md") && paths.contains(&"implement/shots/a.png"));
    let png = list
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "implement/shots/a.png")
        .unwrap();
    assert_eq!(png["kind"], "png");
    assert_eq!(png["stage"], "implement");
    assert_eq!(png["size"], PNG_MAGIC.len());

    let response = get(
        fx.port,
        &format!("/api/runs/{RUN_ID}/files/implement/shots/a.png"),
    );
    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type"), Some("image/png"));
    assert_eq!(response.header("Cache-Control"), Some("no-cache"));
    let mut body = Vec::new();
    response.into_reader().read_to_end(&mut body).unwrap();
    assert_eq!(body, PNG_MAGIC);

    for path in [
        format!("/api/runs/{RUN_ID}/files/../../secret.txt"),
        format!("/api/runs/{RUN_ID}/files/%2e%2e/secret.txt"),
        format!("/api/runs/{RUN_ID}/files/plan%2f..%2f..%2fsecret.txt"),
    ] {
        let answer = raw(fx.port, &path);
        assert!(answer.contains(" 404 "), "{path}: {answer}");
        assert!(!answer.contains("outside"), "{path}");
    }
    assert_eq!(
        get(fx.port, &format!("/api/runs/{RUN_ID}/files/missing.md")).status(),
        404
    );
    assert_eq!(get(fx.port, "/api/runs/nope/files").status(), 404);
    assert_eq!(
        get(fx.port, "/api/runs/nope/files/plan/plan.md").status(),
        404
    );
    fx.handle.stop();
}

#[test]
fn tickets_without_a_flow_have_a_null_flow_summary() {
    let fx = boot();
    let list: Value = get(fx.port, "/api/tickets").into_json().unwrap();
    let row = &list.as_array().unwrap()[0];
    assert_eq!(row["key"], TICKET_KEY);
    assert!(row.get("flowSummary").is_some_and(Value::is_null));
    let detail: Value = get(fx.port, &format!("/api/tickets/{TICKET_KEY}"))
        .into_json()
        .unwrap();
    assert!(detail["ticket"]
        .get("flowSummary")
        .is_some_and(Value::is_null));
    fx.handle.stop();
}
