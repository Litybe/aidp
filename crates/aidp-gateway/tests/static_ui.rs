//! The web UI build served by a real daemon: `/` and client routes answer `index.html`,
//! hashed assets carry their MIME type and a long cache, `/api` stays first, no traversal.

use aidp_gateway::http::{serve_on_with_environment, HttpConfig, ServeHandle};
use aidp_gateway::models::runtime_environment::RuntimeEnvironment;
use aidp_testkit::TempWorkspace;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

const UI_DIR: &str = "dist";
const ASSETS_DIR: &str = "dist/assets";
const INDEX_FILE: &str = "dist/index.html";
const INDEX_BODY: &str = "<!doctype html><title>aidp-studio</title><div id=\"root\"></div>";
const SCRIPT_FILE: &str = "dist/assets/index-d41d8c.js";
const SCRIPT_PATH: &str = "/assets/index-d41d8c.js";
const SCRIPT_BODY: &str = "export const answer = 42;";
const SCRIPT_BROTLI_FILE: &str = "dist/assets/index-d41d8c.js.br";
const BROTLI_BODY: &str = "precompressed brotli bytes";
const HEADER_ACCEPT_ENCODING: &str = "Accept-Encoding";
const HEADER_CONTENT_ENCODING: &str = "Content-Encoding";
const HEADER_VARY: &str = "Vary";
const BROWSER_ENCODINGS: &str = "gzip, deflate, br";
const BROTLI_TOKEN: &str = "br";
const SECRET_FILE: &str = "secret.txt";
const SECRET_BODY: &str = "never served";
const ROOT_PATH: &str = "/";
const DEEP_ROUTE: &str = "/board/GH-12";
const MISSING_ASSET: &str = "/assets/missing-0000.js";
const STATUS_PATH: &str = "/api/gateway/status";
const UNKNOWN_API_PATH: &str = "/api/no-such-route";
const TRAVERSAL_REQUEST: &str =
    "GET /../secret.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
const HEAD_REQUEST: &str = "HEAD / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
const HEADER_CONTENT_TYPE: &str = "Content-Type";
const HEADER_CACHE_CONTROL: &str = "Cache-Control";
const HEADER_CONTENT_LENGTH: &str = "content-length";
const MIME_HTML: &str = "text/html; charset=utf-8";
const MIME_JS: &str = "text/javascript; charset=utf-8";
const MIME_JSON: &str = "application/json";
const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
const CACHE_REVALIDATE: &str = "no-cache";
const NOT_FOUND_MESSAGE: &str = "not found";
const ERROR_FIELD: &str = "error";
const UPTIME_FIELD: &str = "uptimeSec";
const STATUS_OK: u16 = 200;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_LINE_NOT_FOUND: &str = " 404 ";
const STATUS_LINE_OK: &str = " 200 ";
const HEADER_END: &str = "\r\n\r\n";
const IO_TIMEOUT: Duration = Duration::from_secs(10);

fn config(workspace: &TempWorkspace) -> HttpConfig {
    HttpConfig {
        state_dir: workspace.child("state"),
        vault_root: PathBuf::new(),
        hook_token: String::new(),
        ui_token: String::new(),
        read_only: false,
        allowed_origins: vec![],
    }
}

/// A daemon serving a small UI build; the workspace also holds a file outside that build.
fn boot(with_ui: bool) -> (TempWorkspace, ServeHandle, u16) {
    let workspace = TempWorkspace::new("gw-static-ui");
    fs::create_dir_all(workspace.child(ASSETS_DIR)).expect("assets dir");
    fs::write(workspace.child(INDEX_FILE), INDEX_BODY).expect("index");
    fs::write(workspace.child(SCRIPT_FILE), SCRIPT_BODY).expect("script");
    fs::write(workspace.child(SECRET_FILE), SECRET_BODY).expect("secret");
    fs::write(workspace.child(SCRIPT_BROTLI_FILE), BROTLI_BODY).expect("brotli variant");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind port 0");
    let port = listener.local_addr().expect("local addr").port();
    let environment = RuntimeEnvironment {
        ui_dir: with_ui.then(|| workspace.child(UI_DIR)),
        ..Default::default()
    };
    let handle =
        serve_on_with_environment(listener, config(&workspace), environment).expect("serve");
    (workspace, handle, port)
}

fn url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

/// Status, content type, cache policy and body of a GET, error statuses included.
fn get(port: u16, path: &str) -> (u16, String, String, String) {
    let response = match ureq::get(&url(port, path)).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(error) => panic!("GET {path}: {error}"),
    };
    let status = response.status();
    let content_type = response
        .header(HEADER_CONTENT_TYPE)
        .unwrap_or_default()
        .to_string();
    let cache = response
        .header(HEADER_CACHE_CONTROL)
        .unwrap_or_default()
        .to_string();
    let body = response.into_string().expect("body");
    (status, content_type, cache, body)
}

/// Sends `request` verbatim, path included, and returns the whole raw answer.
fn raw(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .expect("read timeout");
    stream.write_all(request.as_bytes()).expect("write");
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

#[test]
fn root_and_client_routes_answer_index() {
    let (_workspace, handle, port) = boot(true);
    for path in [ROOT_PATH, DEEP_ROUTE] {
        let (status, content_type, cache, body) = get(port, path);
        assert_eq!(status, STATUS_OK, "{path}");
        assert_eq!(content_type, MIME_HTML, "{path}");
        assert_eq!(cache, CACHE_REVALIDATE, "{path}");
        assert_eq!(body, INDEX_BODY, "{path}");
    }
    handle.stop();
}

#[test]
fn hashed_asset_has_its_mime_type_and_a_long_cache() {
    let (_workspace, handle, port) = boot(true);
    let (status, content_type, cache, body) = get(port, SCRIPT_PATH);
    assert_eq!(status, STATUS_OK);
    assert_eq!(content_type, MIME_JS);
    assert_eq!(cache, CACHE_IMMUTABLE);
    assert_eq!(body, SCRIPT_BODY);
    let (status, content_type, _, body) = get(port, MISSING_ASSET);
    assert_eq!(status, STATUS_NOT_FOUND);
    assert_eq!(content_type, MIME_JSON);
    let error: Value = serde_json::from_str(&body).expect("json error");
    assert_eq!(error[ERROR_FIELD], NOT_FOUND_MESSAGE);
    handle.stop();
}

#[test]
fn precompressed_variant_is_sent_with_its_encoding() {
    let (_workspace, handle, port) = boot(true);
    let response = ureq::get(&url(port, SCRIPT_PATH))
        .set(HEADER_ACCEPT_ENCODING, BROWSER_ENCODINGS)
        .call()
        .expect("GET with Accept-Encoding");
    assert_eq!(response.status(), STATUS_OK);
    assert_eq!(response.header(HEADER_CONTENT_ENCODING), Some(BROTLI_TOKEN));
    assert_eq!(response.header(HEADER_CONTENT_TYPE), Some(MIME_JS));
    assert_eq!(response.header(HEADER_VARY), Some(HEADER_ACCEPT_ENCODING));
    assert_eq!(response.into_string().expect("body"), BROTLI_BODY);
    let (_, _, _, body) = get(port, SCRIPT_PATH);
    assert_eq!(body, SCRIPT_BODY);
    handle.stop();
}

#[test]
fn api_routes_stay_first() {
    let (_workspace, handle, port) = boot(true);
    let (status, content_type, _, body) = get(port, STATUS_PATH);
    assert_eq!(status, STATUS_OK);
    assert_eq!(content_type, MIME_JSON);
    let payload: Value = serde_json::from_str(&body).expect("status json");
    assert!(payload[UPTIME_FIELD].is_number());
    let (status, content_type, _, _) = get(port, UNKNOWN_API_PATH);
    assert_eq!(status, STATUS_NOT_FOUND);
    assert_eq!(content_type, MIME_JSON);
    handle.stop();
}

#[test]
fn traversal_is_refused_and_head_sends_no_body() {
    let (_workspace, handle, port) = boot(true);
    let answer = raw(port, TRAVERSAL_REQUEST);
    assert!(answer.contains(STATUS_LINE_NOT_FOUND), "{answer}");
    assert!(!answer.contains(SECRET_BODY), "{answer}");
    let answer = raw(port, HEAD_REQUEST);
    assert!(answer.contains(STATUS_LINE_OK), "{answer}");
    let (headers, body) = answer.split_once(HEADER_END).expect("header end");
    assert!(
        headers.to_lowercase().contains(HEADER_CONTENT_LENGTH),
        "{headers}"
    );
    assert!(body.is_empty(), "{body}");
    handle.stop();
}

#[test]
fn without_ui_dir_the_root_is_a_json_404() {
    let (_workspace, handle, port) = boot(false);
    for path in [ROOT_PATH, DEEP_ROUTE, SCRIPT_PATH] {
        let (status, content_type, _, body) = get(port, path);
        assert_eq!(status, STATUS_NOT_FOUND, "{path}");
        assert_eq!(content_type, MIME_JSON, "{path}");
        let error: Value = serde_json::from_str(&body).expect("json error");
        assert_eq!(error[ERROR_FIELD], NOT_FOUND_MESSAGE, "{path}");
    }
    handle.stop();
}

#[test]
fn a_missing_ui_dir_refuses_to_start() {
    let workspace = TempWorkspace::new("gw-static-ui-missing");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind port 0");
    let environment = RuntimeEnvironment {
        ui_dir: Some(workspace.child(UI_DIR)),
        ..Default::default()
    };
    assert!(serve_on_with_environment(listener, config(&workspace), environment).is_err());
}
