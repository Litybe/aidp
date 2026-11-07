//! `GET` and `PUT` of `/api/settings/routing|budget|models` on a really-started daemon.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::TempWorkspace;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;

const ROUTING_FIXTURE: &str = include_str!("fixtures/settings/routing.yaml");
const CONFIG_FIXTURE: &str = include_str!("fixtures/settings/config.yaml");
const BUDGET_PATH: &str = "/api/settings/budget";
const ROUTING_PATH: &str = "/api/settings/routing";
const MODELS_PATH: &str = "/api/settings/models";
const UNKNOWN_PATH: &str = "/api/settings/unknown";
const EVENTS_PATH: &str = "/api/events/recent?limit=500";
const TODO_DIR: &str = "TODO";
const CONFIG_FILE: &str = "config.yaml";
const ROUTING_FILE: &str = "routing.yaml";
const SEEDED_MARKER: &str = ".defaults-seeded";
const EVENT_SETTINGS_CHANGED: &str = "settings.changed";
const STATUS_OK: u16 = 200;
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_NOT_FOUND: u16 = 404;

struct Daemon {
    handle: ServeHandle,
    port: u16,
    vault: PathBuf,
}

impl Daemon {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn call(&self, result: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
        match result {
            Ok(response) => (
                response.status(),
                response.into_json().unwrap_or(Value::Null),
            ),
            Err(ureq::Error::Status(code, response)) => {
                (code, response.into_json().unwrap_or(Value::Null))
            }
            Err(error) => panic!("request failed: {error}"),
        }
    }

    fn get(&self, path: &str) -> (u16, Value) {
        self.call(ureq::get(&self.url(path)).call())
    }

    fn put(&self, path: &str, body: Value) -> (u16, Value) {
        self.call(ureq::put(&self.url(path)).send_json(body))
    }

    fn text(&self, name: &str) -> String {
        fs::read_to_string(self.vault.join(TODO_DIR).join(name)).unwrap()
    }

    fn change_events(&self) -> Vec<Value> {
        let (_, events) = self.get(EVENTS_PATH);
        events
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == EVENT_SETTINGS_CHANGED)
            .map(|event| event["data"].clone())
            .collect()
    }
}

fn boot(tag: &str, read_only: bool) -> Daemon {
    let vault = Box::leak(Box::new(TempWorkspace::new(tag)));
    let vault_root = vault.path().to_path_buf();
    fs::create_dir_all(vault_root.join(TODO_DIR)).unwrap();
    fs::write(vault_root.join(TODO_DIR).join(CONFIG_FILE), CONFIG_FIXTURE).unwrap();
    fs::write(
        vault_root.join(TODO_DIR).join(ROUTING_FILE),
        ROUTING_FIXTURE,
    )
    .unwrap();
    let state = Box::leak(Box::new(TempWorkspace::new(&format!("{tag}-state"))));
    fs::write(state.path().join(SEEDED_MARKER), "").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = serve_on(
        listener,
        HttpConfig {
            state_dir: state.path().to_path_buf(),
            vault_root: vault_root.clone(),
            hook_token: String::new(),
            ui_token: String::new(),
            read_only,
            allowed_origins: Vec::new(),
        },
    )
    .unwrap();
    Daemon {
        handle,
        port,
        vault: vault_root,
    }
}

#[test]
fn budget_get_then_put_writes_the_file_and_publishes_one_event() {
    let daemon = boot("settings-budget", false);

    let (status, before) = daemon.get(BUDGET_PATH);
    assert_eq!(status, STATUS_OK);
    assert_eq!(before["budget"]["per_ticket"]["code"], 3_000_000);
    assert_eq!(before["dispatcher"]["night"]["enabled"], false);

    let body = json!({
        "budget": { "per_ticket": { "doc": "700k" } },
        "dispatcher": { "night": { "enabled": true, "start": "23:30" }, "stop_at_7d_percent": 80 },
    });
    let (status, after) = daemon.put(BUDGET_PATH, body);

    assert_eq!(status, STATUS_OK);
    assert_eq!(after["reloadRequired"], false);
    assert_eq!(after["changed"], true);
    assert_eq!(after["budget"]["per_ticket"]["doc"], 700_000);
    assert_eq!(after["dispatcher"]["night"]["start"], "23:30");
    assert_eq!(
        daemon.get(BUDGET_PATH).1["dispatcher"]["stop_at_7d_percent"],
        80.0
    );
    assert!(daemon
        .text(CONFIG_FILE)
        .contains("# tickets in-progress at once"));
    assert_eq!(daemon.change_events(), vec![json!({ "section": "budget" })]);

    let (_, unchanged) = daemon.put(BUDGET_PATH, json!({ "dispatcher": { "slots": 5 } }));
    assert_eq!(unchanged["changed"], false);
    assert_eq!(daemon.change_events().len(), 1);
    daemon.handle.stop();
}

#[test]
fn a_refused_put_answers_400_with_every_path_and_changes_nothing() {
    let daemon = boot("settings-errors", false);

    let (status, reply) = daemon.put(
        BUDGET_PATH,
        json!({ "budget": { "per_ticket": { "code": "lots" } }, "dispatcher": { "slots": 0 } }),
    );

    assert_eq!(status, STATUS_BAD_REQUEST);
    assert_eq!(reply["code"], "bad_request");
    let paths: Vec<&str> = reply["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| error["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["budget.per_ticket.code", "dispatcher.slots"]);
    assert!(reply["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("token count"));
    assert_eq!(daemon.text(CONFIG_FILE), CONFIG_FIXTURE);
    assert!(daemon.change_events().is_empty());
    daemon.handle.stop();
}

#[test]
fn routing_and_models_round_trip_over_http() {
    let daemon = boot("settings-routing", false);

    let (status, routing) = daemon.get(ROUTING_PATH);
    assert_eq!(status, STATUS_OK);
    assert_eq!(routing["rules"].as_array().unwrap().len(), 10);
    assert_eq!(routing["repos"]["pixelhood-map"]["flow"], "game-map");

    let mut edited = routing.clone();
    edited["repos"]["pixelhood-map"]["flow"] = json!("game-map-v2");
    let (status, saved) = daemon.put(ROUTING_PATH, edited);
    assert_eq!(status, STATUS_OK);
    assert_eq!(saved["repos"]["pixelhood-map"]["flow"], "game-map-v2");
    assert_eq!(
        daemon.get(ROUTING_PATH).1["repos"]["pixelhood-map"]["flow"],
        "game-map-v2"
    );

    let (status, models) = daemon.put(
        MODELS_PATH,
        json!({ "models": { "code": { "report": "haiku" }, "art": { "review": "sonnet" } } }),
    );
    assert_eq!(status, STATUS_OK);
    assert_eq!(models["models"]["art"]["review"], "sonnet");
    assert_eq!(
        daemon.get(MODELS_PATH).1["models"]["code"]["report"],
        "haiku"
    );
    assert_eq!(
        daemon.change_events(),
        vec![
            json!({ "section": "routing" }),
            json!({ "section": "models" })
        ]
    );
    daemon.handle.stop();
}

#[test]
fn read_only_mode_refuses_a_put_and_still_serves_a_get() {
    let daemon = boot("settings-read-only", true);

    let (status, _) = daemon.put(BUDGET_PATH, json!({ "dispatcher": { "slots": 2 } }));
    assert_eq!(status, STATUS_FORBIDDEN);
    assert_eq!(daemon.get(BUDGET_PATH).0, STATUS_OK);
    assert_eq!(daemon.text(CONFIG_FILE), CONFIG_FIXTURE);
    daemon.handle.stop();
}

#[test]
fn an_unknown_section_is_a_404() {
    let daemon = boot("settings-unknown", false);

    assert_eq!(daemon.get(UNKNOWN_PATH).0, STATUS_NOT_FOUND);
    daemon.handle.stop();
}
