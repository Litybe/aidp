//! `POST /api/tickets/<key>/start` validates its overrides before it persists them.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::{EnvGuard, TempWorkspace};
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::store::TicketStore;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

const API_BASE: &str = "/api";
const TICKETS_PREFIX: &str = "/tickets";
const START_SEGMENT: &str = "start";
const VAULT_ENV: &str = "AIDP_VAULT";
const PIPELINES_DIR: &str = "Pipelines";
const TICKET_KEY: &str = "LOC-1";
const ORIGINAL_SERVICE: &str = "svc";
const TARGET_SERVICE: &str = "other";
const BOGUS_TRACK: &str = "bogus";
const STATUS_BAD_REQUEST: u16 = 400;

/// URL of the manual start route of one ticket.
fn start_url(port: u16, key: &str) -> String {
    format!("http://127.0.0.1:{port}{API_BASE}{TICKETS_PREFIX}/{key}/{START_SEGMENT}")
}

/// A local ticket routed to [`ORIGINAL_SERVICE`] in the given status.
fn ticket_in(status: Status) -> Ticket {
    let now = chrono::Utc::now();
    Ticket {
        key: TICKET_KEY.to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "sample".to_string(),
        status,
        priority: Priority::Medium,
        priority_rank: 0,
        service: ORIGINAL_SERVICE.to_string(),
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

/// Seeds a vault holding one ticket and the known target service folder.
fn seed_vault(vault: &Path, status: Status) {
    fs::create_dir_all(vault.join(PIPELINES_DIR).join(TARGET_SERVICE)).unwrap();
    let store = TicketStore::open(vault).unwrap();
    let mut ticket = ticket_in(status);
    store
        .save(&mut ticket, "", &serde_yaml::Mapping::new())
        .unwrap();
}

/// Serves the gateway on a free port with its own state dir.
fn boot_server(vault: &Path) -> (ServeHandle, u16) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-start-state")));
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

/// Posts a start body and returns the status with the JSON reply.
fn post_start(port: u16, body: Value) -> (u16, Value) {
    let resp = ureq::post(&start_url(port, TICKET_KEY))
        .send_json(body)
        .unwrap_or_else(|e| e.into_response().unwrap());
    let status = resp.status();
    (status, resp.into_json().unwrap_or(Value::Null))
}

/// The service the stored note is currently filed under.
fn stored_service(vault: &Path) -> String {
    let (ticket, _, _) = TicketStore::open(vault).unwrap().get(TICKET_KEY).unwrap();
    ticket.service
}

/// A valid start with a service override moves the note to that service.
#[test]
fn service_override_moves_the_note_on_success() {
    let vault = TempWorkspace::new("gw-start-vault");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    seed_vault(vault.path(), Status::Todo);
    let (handle, port) = boot_server(vault.path());

    post_start(port, json!({ "service": TARGET_SERVICE }));

    assert_eq!(stored_service(vault.path()), TARGET_SERVICE);
    handle.stop();
}

/// A start the guard refuses leaves the note in its original service.
#[test]
fn start_refused_by_the_guard_leaves_the_note_in_place() {
    let vault = TempWorkspace::new("gw-start-vault-refused");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    seed_vault(vault.path(), Status::Done);
    let (handle, port) = boot_server(vault.path());

    let (status, _) = post_start(port, json!({ "service": TARGET_SERVICE }));

    assert_ne!(status, 200);
    assert_eq!(stored_service(vault.path()), ORIGINAL_SERVICE);
    handle.stop();
}

/// A track outside quick, standard and deep answers 400 and changes nothing.
#[test]
fn bogus_track_is_a_bad_request() {
    let vault = TempWorkspace::new("gw-start-vault-track");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    seed_vault(vault.path(), Status::Todo);
    let (handle, port) = boot_server(vault.path());

    let (status, _) = post_start(
        port,
        json!({ "service": TARGET_SERVICE, "track": BOGUS_TRACK }),
    );

    assert_eq!(status, STATUS_BAD_REQUEST);
    assert_eq!(stored_service(vault.path()), ORIGINAL_SERVICE);
    handle.stop();
}
