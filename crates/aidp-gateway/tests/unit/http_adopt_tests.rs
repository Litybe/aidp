use super::*;
use aidp_testkit::{EnvGuard, TempWorkspace};
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::store::TicketStore;
use serde_json::json;
use std::fs;

const TICKET_KEY: &str = "SVC-1";
const VAULT_ENV: &str = "AIDP_VAULT";
const SERVICE: &str = "svc";
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_NOT_FOUND: u16 = 404;
const UNKNOWN_KEY: &str = "SVC-404";
const PR_NUMBER: u64 = 61;

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
        autopilot: Autopilot::default(),
        tags: Vec::new(),
    }
}

fn boot_server(vault_root: PathBuf) -> (ServeHandle, u16) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-adopt-state")));
    let state_dir: PathBuf = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir,
        vault_root,
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    (serve_on(listener, config).unwrap(), port)
}

fn post_adopt(port: u16, key: &str, body: Value) -> u16 {
    let url = format!("http://127.0.0.1:{port}/api/tickets/{key}/adopt");
    ureq::post(&url)
        .send_json(body)
        .map(|resp| resp.status())
        .unwrap_or_else(|e| e.into_response().unwrap().status())
}

/// An unknown ticket key answers 404.
#[test]
fn it_adopt_unknown_ticket_is_not_found() {
    let vault = TempWorkspace::new("gw-adopt-vault-unknown");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    TicketStore::open(vault.path()).unwrap();
    let (handle, port) = boot_server(vault.path().to_path_buf());

    let status = post_adopt(port, UNKNOWN_KEY, json!({ "pr": PR_NUMBER }));

    assert_eq!(status, STATUS_NOT_FOUND);
    handle.stop();
}

/// A body without a pull request reference answers 400.
#[test]
fn it_adopt_without_a_pr_reference_is_a_bad_request() {
    let vault = TempWorkspace::new("gw-adopt-vault-blank");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let store = TicketStore::open(vault.path()).unwrap();
    store
        .save(&mut ticket(), "", &serde_yaml::Mapping::new())
        .unwrap();
    let (handle, port) = boot_server(vault.path().to_path_buf());

    let status = post_adopt(port, TICKET_KEY, json!({}));

    assert_eq!(status, STATUS_BAD_REQUEST);
    handle.stop();
}

/// A ticket routed to a service with no project cannot adopt: 404, ticket untouched.
#[test]
fn it_adopt_without_a_project_is_not_found_and_changes_nothing() {
    let vault = TempWorkspace::new("gw-adopt-vault-noproject");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    let store = TicketStore::open(vault.path()).unwrap();
    store
        .save(&mut ticket(), "", &serde_yaml::Mapping::new())
        .unwrap();
    let (handle, port) = boot_server(vault.path().to_path_buf());

    let status = post_adopt(port, TICKET_KEY, json!({ "pr": PR_NUMBER }));

    assert_eq!(status, STATUS_NOT_FOUND);
    let (stored, _, _) = store.get(TICKET_KEY).unwrap();
    assert_eq!(stored.status, Status::Todo);
    handle.stop();
}
