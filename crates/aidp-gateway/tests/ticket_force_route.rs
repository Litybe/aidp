//! `POST /api/tickets/<key>/force` moves a ticket to any column, whatever its status.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::{EnvGuard, TempWorkspace};
use aidp_tickets::model::{Autopilot, IssueType, Phase, Priority, Source, Status, Ticket};
use aidp_tickets::store::TicketStore;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

const VAULT_ENV: &str = "AIDP_VAULT";
const TICKET_KEY: &str = "LOC-7";
const SERVICE: &str = "svc";
const STATUS_OK: u16 = 200;
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_CONFLICT: u16 = 409;
const TARGET_READY: &str = "ready";
const TARGET_TO_TEST: &str = "to-test";
const TARGET_BLOCKED: &str = "blocked";
const TARGET_MERGE: &str = "merge";
const TARGET_UNKNOWN: &str = "shipped";
const FORCED_BLOCKED_REASON: &str = "forced";
const HUMAN_REASON: &str = "stuck since monday";

fn force_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/api/tickets/{TICKET_KEY}/force")
}

fn ticket_in(status: Status, phase: Phase) -> Ticket {
    let now = chrono::Utc::now();
    let mut autopilot = Autopilot::default();
    autopilot.set_phase(phase, now);
    Ticket {
        key: TICKET_KEY.to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "sample".to_string(),
        status,
        priority: Priority::Medium,
        priority_rank: 0,
        service: SERVICE.to_string(),
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
        autopilot,
        tags: Vec::new(),
    }
}

fn seed_vault(vault: &Path, status: Status, phase: Phase) {
    let store = TicketStore::open(vault).unwrap();
    let mut ticket = ticket_in(status, phase);
    store
        .save(&mut ticket, "", &serde_yaml::Mapping::new())
        .unwrap();
}

fn boot_server(vault: &Path) -> (ServeHandle, u16) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-force-state")));
    let state_dir: PathBuf = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir,
        vault_root: vault.to_path_buf(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    (serve_on(listener, config).unwrap(), port)
}

fn post_force(port: u16, body: Value) -> (u16, Value) {
    let resp = ureq::post(&force_url(port))
        .send_json(body)
        .unwrap_or_else(|e| e.into_response().unwrap());
    let status = resp.status();
    (status, resp.into_json().unwrap_or(Value::Null))
}

fn stored(vault: &Path) -> Ticket {
    TicketStore::open(vault).unwrap().get(TICKET_KEY).unwrap().0
}

struct Served {
    vault: TempWorkspace,
    _env: EnvGuard,
    handle: ServeHandle,
    port: u16,
}

fn serve(tag: &str, status: Status, phase: Phase) -> Served {
    let vault = TempWorkspace::new(tag);
    let env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    seed_vault(vault.path(), status, phase);
    let (handle, port) = boot_server(vault.path());
    Served {
        vault,
        _env: env,
        handle,
        port,
    }
}

#[test]
fn ready_moves_a_done_ticket_back_to_todo_in_the_dev_phase() {
    let served = serve("gw-force-ready", Status::Done, Phase::Testing);

    let (status, reply) = post_force(served.port, json!({ "target": TARGET_READY }));

    assert_eq!(status, STATUS_OK, "{reply}");
    assert_eq!(reply["from"], json!(Status::Done.as_str()));
    assert_eq!(reply["to"], json!(Status::Todo.as_str()));
    let ticket = stored(served.vault.path());
    assert_eq!(ticket.status, Status::Todo);
    assert_eq!(ticket.autopilot.phase(), Phase::Dev);
    served.handle.stop();
}

#[test]
fn to_test_leaves_in_progress_without_force_and_puts_the_ticket_in_the_test_lane() {
    let served = serve("gw-force-to-test", Status::Blocked, Phase::Dev);

    let (status, reply) = post_force(served.port, json!({ "target": TARGET_TO_TEST }));

    assert_eq!(status, STATUS_OK, "{reply}");
    let ticket = stored(served.vault.path());
    assert_eq!(ticket.status, Status::InProgress);
    assert_eq!(ticket.autopilot.phase(), Phase::ToTest);
    served.handle.stop();
}

#[test]
fn blocked_records_the_forced_reason() {
    let served = serve("gw-force-blocked", Status::InProgress, Phase::Dev);

    let (status, reply) = post_force(
        served.port,
        json!({ "target": TARGET_BLOCKED, "reason": HUMAN_REASON }),
    );

    assert_eq!(status, STATUS_OK, "{reply}");
    let ticket = stored(served.vault.path());
    assert_eq!(ticket.status, Status::Blocked);
    assert_eq!(
        ticket.autopilot.blocked_reason.as_deref(),
        Some(FORCED_BLOCKED_REASON)
    );
    served.handle.stop();
}

#[test]
fn unknown_target_is_a_bad_request() {
    let served = serve("gw-force-unknown", Status::Todo, Phase::Dev);

    let (status, _) = post_force(served.port, json!({ "target": TARGET_UNKNOWN }));

    assert_eq!(status, STATUS_BAD_REQUEST);
    assert_eq!(stored(served.vault.path()).status, Status::Todo);
    served.handle.stop();
}

#[test]
fn merge_without_a_flow_declaring_the_entry_conflicts_and_changes_nothing() {
    let served = serve("gw-force-merge", Status::InProgress, Phase::ToTest);

    let (status, reply) = post_force(served.port, json!({ "target": TARGET_MERGE }));

    assert_eq!(status, STATUS_CONFLICT, "{reply}");
    let ticket = stored(served.vault.path());
    assert_eq!(ticket.status, Status::InProgress);
    assert_eq!(ticket.autopilot.phase(), Phase::ToTest);
    served.handle.stop();
}
