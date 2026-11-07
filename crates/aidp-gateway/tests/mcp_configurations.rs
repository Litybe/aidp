use aidp_agents::McpConfiguration;
use aidp_gateway::http::{serve_on, HttpConfig};
use aidp_gateway::models::constants::mcp_configuration::INVALID_REQUEST;
use aidp_gateway::models::mcp_configuration_request::McpConfigurationRequest;
use aidp_testkit::{fixtures, EnvGuard, TempWorkspace};
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

const CONFIGURATION_PATH: &str = "/api/runtime/mcp-configs";
const STATUS_PATH: &str = "/api/gateway/status";
const UI_TOKEN: &str = "runtime-mcp-test-token";
const AUTHORIZATION_HEADER: &str = "Authorization";
const BEARER_PREFIX: &str = "Bearer";
const CONFIGURATION_ID: &str = "ada";
const ENVIRONMENT_KEY: &str = "PRIVATE_MCP_VALUE";
const PRIVATE_VALUE: &str = "runtime-only-private-value";
const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";
const HOME_ENV: &str = "HOME";
const USER_PROFILE_ENV: &str = "USERPROFILE";
const OK_STATUS: u16 = 200;
const UNAUTHORIZED_STATUS: u16 = 401;
const BAD_REQUEST_STATUS: u16 = 400;
const CODE_FIELD: &str = "code";
const ERROR_FIELD: &str = "error";
const ID_FIELD: &str = "id";
const UNAUTHORIZED_CODE: &str = "unauthorized";
const BAD_REQUEST_CODE: &str = "bad_request";
const CAPABILITIES_FIELD: &str = "capabilities";
const CAPABILITY_FIELD: &str = "mcpConfigurationReferences";
const INVALID_CONFIGURATION: &str = include_str!("fixtures/mcp/invalid_configuration.json");

fn url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

fn response(result: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
    match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => {
            (response.status(), response.into_json().unwrap())
        }
        Err(error) => panic!("HTTP request failed: {error}"),
    }
}

fn assert_private_value_absent(directory: &Path) {
    for entry in std::fs::read_dir(directory).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            assert_private_value_absent(&path);
        } else {
            assert!(!std::fs::read(path)
                .unwrap()
                .windows(PRIVATE_VALUE.len())
                .any(|bytes| bytes == PRIVATE_VALUE.as_bytes()));
        }
    }
}

#[test]
fn named_configuration_requires_authentication_even_on_loopback_and_never_persists() {
    let workspace = TempWorkspace::new("runtime-mcp-configurations");
    let home = workspace.child("home");
    std::fs::create_dir_all(&home).unwrap();
    let skill = fixtures::skill_root();
    let _environment = EnvGuard::with(&[
        (SKILL_ROOT_ENV, Some(skill.to_str().unwrap())),
        (HOME_ENV, Some(home.to_str().unwrap())),
        (USER_PROFILE_ENV, Some(home.to_str().unwrap())),
    ]);
    let handle = serve_on(
        TcpListener::bind(("127.0.0.1", 0)).unwrap(),
        HttpConfig {
            state_dir: workspace.child("state"),
            vault_root: PathBuf::new(),
            ui_token: UI_TOKEN.to_string(),
            hook_token: String::new(),
            allowed_origins: Vec::new(),
            read_only: false,
        },
    )
    .unwrap();
    let configuration = McpConfigurationRequest {
        id: CONFIGURATION_ID.to_string(),
        configuration: McpConfiguration {
            servers: Vec::new(),
            environment: BTreeMap::from([(ENVIRONMENT_KEY.to_string(), PRIVATE_VALUE.to_string())]),
        },
    };
    let body = serde_json::to_value(configuration).unwrap();
    let (status, refusal) =
        response(ureq::post(&url(handle.port(), CONFIGURATION_PATH)).send_json(body.clone()));
    assert_eq!(status, UNAUTHORIZED_STATUS);
    assert_eq!(refusal[CODE_FIELD], UNAUTHORIZED_CODE);
    assert!(handle
        .app()
        .sessions
        .mcp_configurations()
        .resolve(Some(CONFIGURATION_ID))
        .is_err());

    let authorization = format!("{BEARER_PREFIX} {UI_TOKEN}");
    let (status, registered) = response(
        ureq::post(&url(handle.port(), CONFIGURATION_PATH))
            .set(AUTHORIZATION_HEADER, &authorization)
            .send_json(body),
    );
    assert_eq!(status, OK_STATUS);
    assert_eq!(registered[ID_FIELD], CONFIGURATION_ID);
    assert_eq!(registered.as_object().unwrap().len(), 1);
    assert_eq!(
        handle
            .app()
            .sessions
            .mcp_configurations()
            .resolve(Some(CONFIGURATION_ID))
            .unwrap()
            .unwrap()
            .environment[ENVIRONMENT_KEY],
        PRIVATE_VALUE
    );

    let (status, invalid) = response(
        ureq::post(&url(handle.port(), CONFIGURATION_PATH))
            .set(AUTHORIZATION_HEADER, &authorization)
            .send_string(INVALID_CONFIGURATION),
    );
    assert_eq!(status, BAD_REQUEST_STATUS);
    assert_eq!(invalid[CODE_FIELD], BAD_REQUEST_CODE);
    assert_eq!(invalid[ERROR_FIELD], INVALID_REQUEST);
    assert!(!invalid.to_string().contains(PRIVATE_VALUE));

    let (status, gateway_status) = response(ureq::get(&url(handle.port(), STATUS_PATH)).call());
    assert_eq!(status, OK_STATUS);
    assert_eq!(gateway_status[CAPABILITIES_FIELD][CAPABILITY_FIELD], true);
    assert!(!gateway_status.to_string().contains(PRIVATE_VALUE));
    assert!(!serde_json::to_string(&handle.app().bus.recent(100, 0))
        .unwrap()
        .contains(PRIVATE_VALUE));
    handle.stop();
    assert_private_value_absent(workspace.path());
}
