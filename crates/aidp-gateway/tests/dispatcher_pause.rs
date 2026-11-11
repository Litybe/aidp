//! HTTP contract of the dispatcher pause routes, against a really-started daemon.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::TempWorkspace;
use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;

const PAUSE_PATH: &str = "/dispatcher/pause";
const RESUME_PATH: &str = "/dispatcher/resume";
const DISPATCHER_PATH: &str = "/dispatcher";
const RECENT_EVENTS_PATH: &str = "/events/recent?limit=500";
const TODO_DIR: &str = "TODO";
const CONFIG_FILE: &str = "config.yaml";
const SEEDED_MARKER: &str = ".defaults-seeded";
const INITIAL_CONFIG: &str = "dispatcher:\n  slots: 1\n  paused: false\n";
const PAUSED_LINE: &str = "paused: true";
const UNPAUSED_LINE: &str = "paused: false";
const PAUSED_KEY: &str = "paused";
const EVENT_TYPE_KEY: &str = "type";
const EVENT_DATA_KEY: &str = "data";
const ACTOR_KEY: &str = "actor";
const EVENT_PAUSED_CHANGED: &str = "dispatcher.paused_changed";
const ROLLBACK_ACTOR: &str = "viewer-rollback";
const BODY_PAUSE: &str = r#"{"paused":true}"#;
const BODY_RESUME: &str = r#"{"paused":false}"#;
const BODY_PAUSE_WITH_ACTOR: &str = r#"{"paused":true,"actor":"viewer-rollback"}"#;
const BODY_EMPTY: &str = "";
const BODY_INVALID_JSON: &str = "{not json";
const BODY_STRING_PAUSED: &str = r#"{"paused":"yes"}"#;
const BODY_NO_FIELD: &str = "{}";
const STATUS_OK: u16 = 200;
const STATUS_BAD_REQUEST: u16 = 400;

struct Daemon {
    handle: ServeHandle,
    base: String,
    config: PathBuf,
}

impl Daemon {
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn post(&self, path: &str, body: &str) -> ureq::Response {
        ureq::post(&self.url(path))
            .set("Content-Type", "application/json")
            .send_string(body)
            .unwrap_or_else(|error| error.into_response().expect("transport error"))
    }

    fn get_json(&self, path: &str) -> Value {
        ureq::get(&self.url(path))
            .call()
            .expect("get")
            .into_json()
            .expect("json body")
    }

    fn config_text(&self) -> String {
        fs::read_to_string(&self.config).expect("read config.yaml")
    }

    fn paused_events(&self) -> Vec<Value> {
        let events = self.get_json(RECENT_EVENTS_PATH);
        events
            .as_array()
            .expect("events array")
            .iter()
            .filter(|event| event[EVENT_TYPE_KEY] == EVENT_PAUSED_CHANGED)
            .cloned()
            .collect()
    }
}

fn boot(tag: &str) -> Daemon {
    let vault = Box::leak(Box::new(TempWorkspace::new(tag)));
    let vault_root = vault.path().to_path_buf();
    fs::create_dir_all(vault_root.join(TODO_DIR)).unwrap();
    let config = vault_root.join(TODO_DIR).join(CONFIG_FILE);
    fs::write(&config, INITIAL_CONFIG).unwrap();
    let state = Box::leak(Box::new(TempWorkspace::new(&format!("{tag}-state"))));
    let state_dir = state.path().to_path_buf();
    fs::write(state_dir.join(SEEDED_MARKER), "").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = listener.local_addr().unwrap().port();
    let handle = serve_on(
        listener,
        HttpConfig {
            state_dir,
            vault_root,
            hook_token: String::new(),
            ui_token: String::new(),
            read_only: false,
            allowed_origins: Vec::new(),
        },
    )
    .expect("serve_on failed");
    Daemon {
        handle,
        base: format!("http://127.0.0.1:{port}/api"),
        config,
    }
}

fn assert_paused(daemon: &Daemon, expected: bool) {
    assert_eq!(
        daemon.get_json(DISPATCHER_PATH)[PAUSED_KEY],
        Value::Bool(expected)
    );
}

#[test]
fn pause_then_resume_toggles_and_returns_effective_state() {
    let daemon = boot("dp-toggle");
    let paused = daemon.post(PAUSE_PATH, BODY_PAUSE);
    assert_eq!(paused.status(), STATUS_OK);
    assert_eq!(
        paused.into_json::<Value>().unwrap()[PAUSED_KEY],
        Value::Bool(true)
    );
    assert_paused(&daemon, true);
    assert!(daemon.config_text().contains(PAUSED_LINE));

    let resumed = daemon.post(PAUSE_PATH, BODY_RESUME);
    assert_eq!(resumed.status(), STATUS_OK);
    assert_eq!(
        resumed.into_json::<Value>().unwrap()[PAUSED_KEY],
        Value::Bool(false)
    );
    assert_paused(&daemon, false);
    assert!(daemon.config_text().contains(UNPAUSED_LINE));
    daemon.handle.stop();
}

#[test]
fn invalid_bodies_answer_400_and_leave_the_pause_unchanged() {
    let daemon = boot("dp-invalid");
    for body in [BODY_INVALID_JSON, BODY_STRING_PAUSED] {
        let reply = daemon.post(PAUSE_PATH, body);
        assert_eq!(reply.status(), STATUS_BAD_REQUEST, "body: {body:?}");
        assert_paused(&daemon, false);
        assert_eq!(daemon.config_text(), INITIAL_CONFIG);
    }
    daemon.handle.stop();
}

#[test]
fn empty_or_field_less_body_pauses() {
    for body in [BODY_EMPTY, BODY_NO_FIELD] {
        let daemon = boot("dp-default");
        assert_eq!(
            daemon.post(PAUSE_PATH, body).status(),
            STATUS_OK,
            "body: {body:?}"
        );
        assert_paused(&daemon, true);
        daemon.handle.stop();
    }
}

#[test]
fn repeated_identical_call_returns_200_without_a_second_event() {
    let daemon = boot("dp-repeat");
    assert_eq!(
        daemon.post(PAUSE_PATH, BODY_PAUSE_WITH_ACTOR).status(),
        STATUS_OK
    );
    let again = daemon.post(PAUSE_PATH, BODY_PAUSE_WITH_ACTOR);
    assert_eq!(again.status(), STATUS_OK);
    assert_eq!(
        again.into_json::<Value>().unwrap()[PAUSED_KEY],
        Value::Bool(true)
    );
    let events = daemon.paused_events();
    assert_eq!(events.len(), 1, "events: {events:?}");
    assert_eq!(events[0][EVENT_DATA_KEY][ACTOR_KEY], ROLLBACK_ACTOR);
    assert_eq!(events[0][EVENT_DATA_KEY][PAUSED_KEY], Value::Bool(true));
    daemon.handle.stop();
}

#[test]
fn resume_route_returns_the_unpaused_state() {
    let daemon = boot("dp-resume");
    assert_eq!(daemon.post(PAUSE_PATH, BODY_PAUSE).status(), STATUS_OK);
    let reply = daemon.post(RESUME_PATH, BODY_EMPTY);
    assert_eq!(reply.status(), STATUS_OK);
    assert_eq!(
        reply.into_json::<Value>().unwrap()[PAUSED_KEY],
        Value::Bool(false)
    );
    assert_paused(&daemon, false);
    daemon.handle.stop();
}
