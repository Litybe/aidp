//! With `AIDP_VAULT` set to a disabling value, the gateway serves the no-vault
//! answers and never creates a directory named after the value.

use aidp_gateway::http::{serve_on, HttpConfig};
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;

const VAULT_ENV: &str = "AIDP_VAULT";
const GATEWAY_DIR_ENV: &str = "AIDP_GATEWAY_DIR";
const DISABLING_VALUES: [&str; 6] = ["off", "none", "0", "false", "-", "OFF"];
const STATUS_PATH: &str = "/api/gateway/status";
const PLAN_PATH: &str = "/api/dispatcher/plan";
const TICKETS_PATH: &str = "/api/tickets";
const NO_VAULT_MODE: &str = "no-vault";
const NO_VAULT_ERROR: &str = "no vault configured";
const UNAVAILABLE_STATUS: u16 = 503;

fn url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

fn get_json(port: u16, path: &str) -> (u16, Value) {
    match ureq::get(&url(port, path)).call() {
        Ok(resp) => (resp.status(), resp.into_json().expect("invalid JSON")),
        Err(ureq::Error::Status(code, resp)) => (code, resp.into_json().expect("invalid JSON")),
        Err(err) => panic!("GET {path} failed: {err}"),
    }
}

fn check_disabling_value(value: &str) {
    let workspace = TempWorkspace::new("gw-vault-off");
    let state_dir = workspace.path().to_path_buf();
    fs::write(state_dir.join(".defaults-seeded"), "").expect("seed marker");
    let state_dir_text = state_dir.display().to_string();
    let _env = EnvGuard::with(&[
        (VAULT_ENV, Some(value)),
        (GATEWAY_DIR_ENV, Some(state_dir_text.as_str())),
    ]);

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    let handle = serve_on(listener, config).expect("serve_on failed");
    let port = handle.port();

    let (status, plan) = get_json(port, PLAN_PATH);
    assert_eq!(status, 200, "plan for {value}");
    assert_eq!(plan["mode"].as_str(), Some(NO_VAULT_MODE));
    assert_eq!(plan["launches"].as_array().map(Vec::len), Some(0));
    assert_eq!(plan["skipped"].as_array().map(Vec::len), Some(0));

    let (status, gateway_status) = get_json(port, STATUS_PATH);
    assert_eq!(status, 200, "status for {value}");
    assert!(gateway_status["vault"].is_null(), "vault for {value}");

    let (status, tickets) = get_json(port, TICKETS_PATH);
    assert_eq!(status, UNAVAILABLE_STATUS, "tickets for {value}");
    assert_eq!(tickets["error"].as_str(), Some(NO_VAULT_ERROR));

    handle.stop();

    let cwd = std::env::current_dir().expect("cwd");
    assert!(!cwd.join(value).exists(), "{value} created under cwd");
    assert!(
        !state_dir.join(value).exists(),
        "{value} created under state dir"
    );
}

#[test]
fn disabling_values_yield_no_vault_answers() {
    for value in DISABLING_VALUES {
        check_disabling_value(value);
    }
}
