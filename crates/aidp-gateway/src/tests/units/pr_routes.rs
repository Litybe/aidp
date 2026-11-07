//! PR status routes and webhook-driven cache invalidation.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::{EnvGuard, ProcShim, TempWorkspace};
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::store::TicketStore;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

const VAULT_ENV: &str = "AIDP_VAULT";
const HOOK_TOKEN: &str = "hook-secret";
const TICKET_KEY: &str = "LOC-99";
const SERVICE: &str = "svc";
const PIPELINES_DIR: &str = "Pipelines";
const NO_PR_REASON: &str = "no-pull-request";
const EVENT_HEADER: &str = "X-GitHub-Event";
const SIGNATURE_HEADER: &str = "X-Hub-Signature-256";
const EVENT_CHECK_RUN: &str = "check_run";
const PR_STATUS_EVENT: &str = "pr.status";
const UNKNOWN_AUTOMATION: &str = "no-such-automation";
const REPO: &str = "acme/widgets";
const PR_NUMBER: u64 = 42;
const STATUS_OK: u16 = 200;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_BAD_REQUEST: u16 = 400;
const DEFAULTS_SEEDED_FILE: &str = ".defaults-seeded";
const LOOPBACK_HOST: &str = "127.0.0.1";
const PROJECTS_FILE: &str = "projects.json";
const GH: &str = "gh";
const GRAPHQL_ARG: &str = "graphql";
const COMPARE_SEGMENT: &str = "compare";
const CLOSED_PAYLOAD: &str = include_str!("fixtures/prs/graphql_closed.json");
const NO_PR_PAYLOAD: &str = "{}";
const BEHIND_BY_KEY: &str = "behindBy";
const NO_PROJECT_MESSAGE: &str = "no project for service svc";

/// Base URL of the served gateway.
fn base(port: u16) -> String {
    format!("http://{LOOPBACK_HOST}:{port}/api")
}

/// Serves the gateway on a free port with its own state dir.
fn boot_server() -> (ServeHandle, u16) {
    boot_server_with_projects(&[], &PathBuf::new())
}

/// Serves the gateway with one registered project per `(name, path)` pair.
fn boot_server_with_projects(projects: &[(&str, &Path)], vault: &Path) -> (ServeHandle, u16) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-pr-state")));
    let state_dir: PathBuf = workspace.path().to_path_buf();
    let entries: Vec<Value> = projects
        .iter()
        .map(|(name, path)| json!({ "id": name, "name": name, "path": path.to_str().unwrap() }))
        .collect();
    if !entries.is_empty() {
        fs::write(
            state_dir.join(PROJECTS_FILE),
            serde_json::to_string(&entries).unwrap(),
        )
        .unwrap();
    }
    fs::write(state_dir.join(DEFAULTS_SEEDED_FILE), "").unwrap();
    let listener = TcpListener::bind((LOOPBACK_HOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir,
        vault_root: vault.to_path_buf(),
        hook_token: HOOK_TOKEN.to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    (serve_on(listener, config).unwrap(), port)
}

/// Seeds a vault holding one local ticket with no recorded PR.
fn seed_vault(vault: &Path) {
    fs::create_dir_all(vault.join(PIPELINES_DIR).join(SERVICE)).unwrap();
    let now = chrono::Utc::now();
    let mut ticket = Ticket {
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
    };
    TicketStore::open(vault)
        .unwrap()
        .save(&mut ticket, "", &serde_yaml::Mapping::new())
        .unwrap();
}

/// GET returning the status and JSON body, error statuses included.
fn get(url: &str) -> (u16, Value) {
    let resp = ureq::get(url)
        .call()
        .unwrap_or_else(|e| e.into_response().unwrap());
    let status = resp.status();
    (status, resp.into_json().unwrap_or(Value::Null))
}

/// A check_run delivery naming one PR of [`REPO`].
fn check_run_payload() -> String {
    json!({
        "repository": { "full_name": REPO },
        "check_run": { "pull_requests": [ { "number": PR_NUMBER } ] },
    })
    .to_string()
}

/// `sha256=<hex>` HMAC of `payload` under the hook token, as GitHub signs it.
fn signature(payload: &str) -> String {
    format!(
        "sha256={}",
        aidp_gateway::crypto::hmac_sha256_hex(HOOK_TOKEN.as_bytes(), payload.as_bytes())
    )
}

/// A ticket whose branch has no PR answers 404 with the machine-read reason.
#[test]
fn it_ticket_without_a_pull_request_answers_not_found_with_reason() {
    let vault = TempWorkspace::new("gw-pr-vault");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    seed_vault(vault.path());
    let workspace = TempWorkspace::new("gw-pr-shim-no-pr");
    let shim = ProcShim::new(GH, &workspace);
    shim.set_stdout(NO_PR_PAYLOAD);
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (handle, port) = boot_server_with_projects(&[(SERVICE, project_dir)], vault.path());

    let (status, body) = get(&format!("{}/tickets/{TICKET_KEY}/pr", base(port)));

    assert_eq!(status, STATUS_NOT_FOUND);
    assert_eq!(body["reason"], NO_PR_REASON);
    handle.stop();
}

/// A ticket whose service has no registered project answers 404 instead of probing the cwd.
#[test]
fn it_ticket_without_a_registered_project_answers_not_found() {
    let vault = TempWorkspace::new("gw-pr-vault-noproj");
    let _env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
    seed_vault(vault.path());
    let (handle, port) = boot_server_with_projects(&[], vault.path());

    let (status, body) = get(&format!("{}/tickets/{TICKET_KEY}/pr", base(port)));

    assert_eq!(status, STATUS_NOT_FOUND);
    assert_eq!(body["error"], NO_PROJECT_MESSAGE);
    handle.stop();
}

/// A status request without a number is rejected before any lookup.
#[test]
fn it_status_without_a_number_is_a_bad_request() {
    let (handle, port) = boot_server();

    let (status, _) = get(&format!("{}/prs/status?repo={REPO}", base(port)));

    assert_eq!(status, STATUS_BAD_REQUEST);
    handle.stop();
}

/// A signed check_run delivery invalidates the PR even when no automation matches the id.
#[test]
fn it_signed_webhook_for_unknown_automation_still_invalidates() {
    let (handle, port) = boot_server();
    let payload = check_run_payload();

    let _ = ureq::post(&format!("{}/hooks/{UNKNOWN_AUTOMATION}", base(port)))
        .set(EVENT_HEADER, EVENT_CHECK_RUN)
        .set(SIGNATURE_HEADER, &signature(&payload))
        .set("Content-Type", "application/json")
        .send_string(&payload);

    let (_, events) = get(&format!("{}/events/recent", base(port)));
    let emitted = events
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["type"] == PR_STATUS_EVENT && event["data"]["number"] == PR_NUMBER);
    assert!(emitted, "no pr.status event in {events}");
    handle.stop();
}

/// Two status reads inside the cache window reach `gh` for one GraphQL query only.
#[test]
fn it_two_status_reads_within_the_ttl_call_gh_graphql_once() {
    let workspace = TempWorkspace::new("gw-pr-shim-ttl");
    let shim = ProcShim::new(GH, &workspace);
    shim.set_stdout(CLOSED_PAYLOAD);
    let (handle, port) = boot_server();
    let url = format!("{}/prs/status?repo={REPO}&number={PR_NUMBER}", base(port));

    let (first_status, _) = get(&url);
    let (second_status, _) = get(&url);

    let graphql_calls = shim
        .calls()
        .iter()
        .filter(|call| call.iter().any(|arg| arg == GRAPHQL_ARG))
        .count();
    assert_eq!((first_status, second_status), (STATUS_OK, STATUS_OK));
    assert_eq!(graphql_calls, 1, "calls: {:?}", shim.calls());
    handle.stop();
}

/// A closed PR is never compared against its base and carries no `behindBy`.
#[test]
fn it_closed_pull_request_skips_compare_and_omits_behind_by() {
    let workspace = TempWorkspace::new("gw-pr-shim-closed");
    let shim = ProcShim::new(GH, &workspace);
    shim.set_stdout(CLOSED_PAYLOAD);
    let (handle, port) = boot_server();

    let (status, body) = get(&format!(
        "{}/prs/status?repo={REPO}&number={PR_NUMBER}",
        base(port)
    ));

    let compared = shim
        .calls()
        .iter()
        .flatten()
        .any(|arg| arg.contains(COMPARE_SEGMENT));
    assert_eq!(status, STATUS_OK, "body: {body} calls: {:?}", shim.calls());
    assert!(!compared, "calls: {:?}", shim.calls());
    assert!(body.get(BEHIND_BY_KEY).is_none(), "body: {body}");
    handle.stop();
}
