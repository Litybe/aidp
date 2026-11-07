//! `GET/POST /api/prs/comments/state` against a really-started gateway.

use crate::http::{serve_on, HttpConfig, ServeHandle};
use crate::models::comment_state_request::CommentStateRequest;
use crate::models::enums::point_state::PointState;
use aidp_testkit::TempWorkspace;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;

const REPO: &str = "o/r";
const NUMBER: u64 = 123;
const DEFAULTS_SEEDED_FILE: &str = ".defaults-seeded";
const LOOPBACK: &str = "127.0.0.1";
const POINT_ID: &str = "pt-1";
const OK: u16 = 200;

const STATE_ROUTE: &str = "/api/prs/comments/state";

struct Daemon {
    handle: Option<ServeHandle>,
    port: u16,
}

impl Daemon {
    fn url(&self, path: &str) -> String {
        format!("http://{LOOPBACK}:{}{path}", self.port)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
        }
    }
}

fn boot_daemon(vault_root: PathBuf) -> Daemon {
    let state_dir = Box::leak(Box::new(TempWorkspace::new("gw-pr-comments-state")))
        .path()
        .to_path_buf();
    fs::write(state_dir.join(DEFAULTS_SEEDED_FILE), "").unwrap();
    let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root,
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    Daemon {
        handle: Some(serve_on(listener, config).unwrap()),
        port,
    }
}

fn answer(result: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
    let response = result.unwrap_or_else(|error| error.into_response().expect("transport error"));
    (
        response.status(),
        response.into_json().unwrap_or(Value::Null),
    )
}

fn post<B: Serialize>(daemon: &Daemon, path: &str, body: B) -> (u16, Value) {
    answer(ureq::post(&daemon.url(path)).send_json(serde_json::to_value(body).unwrap()))
}

fn get(daemon: &Daemon, path: &str) -> (u16, Value) {
    answer(ureq::get(&daemon.url(path)).call())
}

fn state_url() -> String {
    format!("/api/prs/comments/state?repo={REPO}&number={NUMBER}")
}

fn state_body(state: PointState) -> CommentStateRequest {
    CommentStateRequest {
        repo: REPO.to_string(),
        number: NUMBER,
        point_id: POINT_ID.to_string(),
        state,
    }
}

#[test]
fn it_pr_comment_state_round_trips_without_touching_github() {
    let daemon = boot_daemon(PathBuf::new());
    let (status, empty) = get(&daemon, &state_url());
    assert_eq!(status, OK);
    assert!(empty["handed"].as_array().unwrap().is_empty());
    let (status, set) = post(&daemon, STATE_ROUTE, state_body(PointState::Ignored));
    assert_eq!(status, OK);
    assert_eq!(set["points"][POINT_ID], "ignored");
    assert_eq!(get(&daemon, &state_url()).1["points"][POINT_ID], "ignored");
    let (_, cleared) = post(&daemon, STATE_ROUTE, state_body(PointState::None));
    assert!(cleared["points"].as_object().unwrap().is_empty());
}
