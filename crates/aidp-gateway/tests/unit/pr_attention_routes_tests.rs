//! HTTP contract of the PR attention routes against a really-started daemon.

use crate::http::{serve_on, HttpConfig, ServeHandle};
use crate::models::attention_list_response::AttentionListResponse;
use crate::models::attention_read_request::AttentionReadRequest;
use crate::models::attention_read_response::AttentionReadResponse;
use crate::models::enums::attention_signal_kind::AttentionSignalKind;
use crate::models::pr_attention_file::PrAttentionFile;
use crate::models::pr_attention_record::PrAttentionRecord;
use crate::pr_attention::ATTENTION_FILE;
use aidp_testkit::TempWorkspace;
use std::fs;
use std::net::TcpListener;
use std::path::Path;

const LOOPBACK: &str = "127.0.0.1";
const HOOK_TOKEN: &str = "hook-secret";
const UI_TOKEN: &str = "ui-secret";
const LIST_PATH: &str = "/api/prs/attention";
const UNREAD_PATH: &str = "/api/prs/attention?unread=true";
const READ_PATH: &str = "/api/prs/attention/read";
const READ_FINGERPRINT: &str = "fp-read";
const UNREAD_FINGERPRINT: &str = "fp-unread";
const REPO: &str = "pixelhood/service";
const HEAD_SHA: &str = "abc123";
const FIRST_SEEN: &str = "2026-10-05T08:00:00Z";
const INVALID_JSON: &str = "{not json";
const STATUS_OK: u16 = 200;
const STATUS_BAD_REQUEST: u16 = 400;

fn record(fingerprint: &str, number: u64, read: bool) -> PrAttentionRecord {
    PrAttentionRecord {
        fingerprint: fingerprint.to_string(),
        repo: REPO.to_string(),
        number,
        signal: AttentionSignalKind::NewComment,
        head_sha: HEAD_SHA.to_string(),
        detail: None,
        first_seen: FIRST_SEEN.to_string(),
        notified: true,
        read,
    }
}

fn seed(state_dir: &Path) {
    let mut stored = PrAttentionFile::default();
    for entry in [
        record(READ_FINGERPRINT, 1, true),
        record(UNREAD_FINGERPRINT, 2, false),
    ] {
        stored.records.insert(entry.fingerprint.clone(), entry);
    }
    fs::write(
        state_dir.join(ATTENTION_FILE),
        serde_json::to_string(&stored).expect("serialize store"),
    )
    .expect("write store");
}

fn boot(state_dir: &Path) -> (ServeHandle, u16) {
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let config = HttpConfig {
        state_dir: state_dir.to_path_buf(),
        vault_root: Default::default(),
        hook_token: HOOK_TOKEN.to_string(),
        ui_token: UI_TOKEN.to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    (serve_on(listener, config).expect("serve_on"), port)
}

fn url(port: u16, path: &str) -> String {
    format!("http://{LOOPBACK}:{port}{path}")
}

fn list(port: u16, path: &str) -> Vec<PrAttentionRecord> {
    let response = ureq::get(&url(port, path)).call().expect("GET list");
    assert_eq!(response.status(), STATUS_OK);
    response
        .into_json::<AttentionListResponse>()
        .expect("list body")
        .items
}

fn post_status(port: u16, body: &str) -> u16 {
    let result = ureq::post(&url(port, READ_PATH))
        .set("Content-Type", "application/json")
        .send_string(body);
    match result {
        Ok(response) => response.status(),
        Err(ureq::Error::Status(code, _)) => code,
        Err(error) => panic!("transport error: {error}"),
    }
}

fn post_json(port: u16, body: &impl serde::Serialize) -> ureq::Response {
    ureq::post(&url(port, READ_PATH))
        .send_json(body)
        .expect("POST read")
}

#[test]
fn it_attention_routes_list_filter_mark_and_persist() {
    let workspace = TempWorkspace::new("gw-attention");
    let state_dir = workspace.path().to_path_buf();
    seed(&state_dir);

    let (handle, port) = boot(&state_dir);
    assert_eq!(list(port, LIST_PATH).len(), 2);
    let unread = list(port, UNREAD_PATH);
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].fingerprint, UNREAD_FINGERPRINT);

    let marked = post_json(
        port,
        &AttentionReadRequest {
            fingerprints: Some(vec![UNREAD_FINGERPRINT.to_string()]),
            all: None,
        },
    );
    assert_eq!(marked.status(), STATUS_OK);
    let count = marked
        .into_json::<AttentionReadResponse>()
        .expect("read body")
        .count;
    assert_eq!(count, 1);
    assert!(list(port, UNREAD_PATH).is_empty());

    let all = post_json(
        port,
        &AttentionReadRequest {
            fingerprints: None,
            all: Some(true),
        },
    );
    assert_eq!(all.status(), STATUS_OK);
    assert_eq!(post_status(port, "{}"), STATUS_BAD_REQUEST);
    assert_eq!(post_status(port, INVALID_JSON), STATUS_BAD_REQUEST);
    handle.stop();

    let (restarted, restarted_port) = boot(&state_dir);
    let persisted = list(restarted_port, LIST_PATH);
    assert_eq!(persisted.len(), 2);
    assert!(persisted.iter().all(|entry| entry.read));
    assert!(list(restarted_port, UNREAD_PATH).is_empty());
    restarted.stop();
}
