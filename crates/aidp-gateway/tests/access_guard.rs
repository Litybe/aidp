//! LOC-26 access guard: Origin allowlist (CSRF), CORS echo, header-only tokens,
//! single-use SSE tokens and GitHub-signed webhooks, against a really-started daemon.
//!
//! Off-loopback scenarios bind the daemon on this host's non-loopback address (found
//! with a connected UDP socket, which sends nothing) and call it on that address, so
//! the daemon sees a non-loopback peer. When the host has no such address, those
//! tests print a notice and return.

use aidp_gateway::crypto::hmac_sha256_hex;
use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::TempWorkspace;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

const ALLOWED: &str = "http://127.0.0.1:5173";
const EVIL: &str = "https://evil.example";
const METHODS: &str = "GET, POST, PUT, PATCH, DELETE, OPTIONS";

struct Daemon {
    handle: ServeHandle,
    base: String,
    host: String,
}

impl Daemon {
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }
}

fn state_dir(tag: &str) -> PathBuf {
    let ws = Box::leak(Box::new(TempWorkspace::new(tag)));
    let dir = ws.path().to_path_buf();
    fs::write(dir.join(".defaults-seeded"), "").expect("seed marker");
    dir
}

/// This host's non-loopback IPv4, if any. `connect` on UDP only picks a route.
fn non_loopback_ip() -> Option<IpAddr> {
    if std::env::var("AIDP_TEST_LAN").as_deref() != Ok("1") {
        return None;
    }
    let s = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    s.connect(("192.0.2.1", 9)).ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn boot(ip: IpAddr, tag: &str, vault_root: PathBuf, hook: &str, ui: &str) -> Option<Daemon> {
    let listener = TcpListener::bind((ip, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let state_dir = state_dir(tag);
    let config = HttpConfig {
        state_dir,
        vault_root,
        hook_token: hook.to_string(),
        ui_token: ui.to_string(),
        read_only: false,
        allowed_origins: vec![ALLOWED.to_string()],
    };
    let handle = serve_on(listener, config).expect("serve_on failed");
    let host = format!("{ip}:{port}");
    Some(Daemon {
        handle,
        base: format!("http://{host}/api"),
        host,
    })
}

fn boot_loopback(tag: &str, vault_root: PathBuf, hook: &str, ui: &str) -> Daemon {
    boot(IpAddr::from([127, 0, 0, 1]), tag, vault_root, hook, ui).expect("loopback bind")
}

fn boot_remote(tag: &str, hook: &str, ui: &str) -> Option<Daemon> {
    let Some(ip) = non_loopback_ip() else {
        eprintln!(
            "skipped: LAN tests are off (set AIDP_TEST_LAN=1 to bind a non-loopback interface) or no such interface"
        );
        return None;
    };
    boot(ip, tag, PathBuf::new(), hook, ui)
}

fn call(r: ureq::Request) -> ureq::Response {
    r.call()
        .unwrap_or_else(|e| e.into_response().expect("transport error"))
}

fn send(r: ureq::Request, body: &str) -> ureq::Response {
    r.set("Content-Type", "application/json")
        .send_string(body)
        .unwrap_or_else(|e| e.into_response().expect("transport error"))
}

fn vault_with_config(tag: &str) -> (PathBuf, PathBuf) {
    let ws = Box::leak(Box::new(TempWorkspace::new(tag)));
    let root = ws.path().to_path_buf();
    fs::create_dir_all(root.join("TODO")).unwrap();
    let cfg = root.join("TODO").join("config.yaml");
    fs::write(&cfg, "dispatcher:\n  slots: 1\n  paused: false\n").unwrap();
    (root, cfg)
}

fn read(p: &Path) -> Vec<u8> {
    fs::read(p).expect("read config.yaml")
}

// 1. A mutating request with a foreign Origin is refused, even from loopback.
#[test]
fn foreign_origin_post_is_403_and_writes_nothing() {
    let (vault, cfg) = vault_with_config("ag-1");
    let d = boot_loopback("ag-1s", vault, "", "");
    let before = read(&cfg);
    let r = send(
        ureq::post(&d.url("/dispatcher/pause")).set("Origin", EVIL),
        "",
    );
    assert_eq!(r.status(), 403);
    assert_eq!(read(&cfg), before, "config.yaml must be untouched");
    d.handle.stop();
}

// 2. Without Origin, the same request goes through and persists.
#[test]
fn no_origin_post_pauses_dispatcher() {
    let (vault, cfg) = vault_with_config("ag-2");
    let d = boot_loopback("ag-2s", vault, "", "");
    let r = send(
        ureq::post(&d.url("/dispatcher/pause")),
        r#"{"paused":true}"#,
    );
    assert_eq!(r.status(), 200);
    let text = String::from_utf8(read(&cfg)).unwrap();
    assert!(text.contains("paused: true"), "config.yaml was: {text}");
    d.handle.stop();
}

// 3. An allowlisted Origin is accepted and echoed exactly.
#[test]
fn allowed_origin_post_is_echoed() {
    let (vault, _) = vault_with_config("ag-3");
    let d = boot_loopback("ag-3s", vault, "", "");
    let r = send(
        ureq::post(&d.url("/dispatcher/resume")).set("Origin", ALLOWED),
        "",
    );
    assert_eq!(r.status(), 200);
    assert_eq!(r.header("Access-Control-Allow-Origin"), Some(ALLOWED));
    d.handle.stop();
}

// 4. A foreign Origin on a GET is served, but without any Allow-Origin.
#[test]
fn foreign_origin_get_has_no_allow_origin() {
    let d = boot_loopback("ag-4", PathBuf::new(), "", "");
    let r = call(ureq::get(&d.url("/gateway/status")).set("Origin", EVIL));
    assert_eq!(r.status(), 200);
    assert_eq!(r.header("Access-Control-Allow-Origin"), None);
    d.handle.stop();
}

// 5. Preflight: bare 204 for a foreign Origin, full CORS for the allowed one.
#[test]
fn preflight_depends_on_origin() {
    let d = boot_loopback("ag-5", PathBuf::new(), "", "");
    let evil = call(
        ureq::request("OPTIONS", &d.url("/sessions"))
            .set("Origin", EVIL)
            .set("Access-Control-Request-Method", "POST"),
    );
    assert_eq!(evil.status(), 204);
    assert_eq!(evil.header("Access-Control-Allow-Methods"), None);
    assert_eq!(evil.header("Access-Control-Allow-Origin"), None);

    let ok = call(
        ureq::request("OPTIONS", &d.url("/sessions"))
            .set("Origin", ALLOWED)
            .set("Access-Control-Request-Method", "POST"),
    );
    assert_eq!(ok.header("Access-Control-Allow-Methods"), Some(METHODS));
    assert_eq!(ok.header("Access-Control-Allow-Origin"), Some(ALLOWED));
    d.handle.stop();
}

// No Origin: never a wildcard, Allow-Methods still present.
#[test]
fn no_origin_response_has_methods_but_no_wildcard() {
    let d = boot_loopback("ag-nw", PathBuf::new(), "", "");
    let r = call(ureq::get(&d.url("/gateway/status")));
    assert_eq!(r.status(), 200);
    assert_ne!(r.header("Access-Control-Allow-Origin"), Some("*"));
    assert_eq!(r.header("Access-Control-Allow-Methods"), Some(METHODS));
    d.handle.stop();
}

// 6. The UI token is never read from the query string.
#[test]
fn query_token_is_ignored_off_loopback() {
    let Some(d) = boot_remote("ag-6", "", "s3cr3t") else {
        return;
    };
    let r = call(ureq::get(&d.url("/gateway/status?token=s3cr3t")));
    assert_eq!(r.status(), 401);
    let body: Value = r.into_json().unwrap();
    assert_eq!(
        body,
        json!({ "error": "unauthorized", "code": "unauthorized" })
    );
    d.handle.stop();
}

// 7. Header tokens are accepted off-loopback.
#[test]
fn header_tokens_are_accepted_off_loopback() {
    let Some(d) = boot_remote("ag-7", "", "s3cr3t") else {
        return;
    };
    let x = call(ureq::get(&d.url("/gateway/status")).set("X-Aidp-Token", "s3cr3t"));
    assert_eq!(x.status(), 200);
    let b = call(ureq::get(&d.url("/gateway/status")).set("Authorization", "Bearer s3cr3t"));
    assert_eq!(b.status(), 200);
    d.handle.stop();
}

fn seed_webhook_automation(d: &Daemon) {
    let r = send(
        ureq::post(&d.url("/automations")),
        &json!({
            "id": "demo",
            "name": "demo",
            "enabled": true,
            "trigger": { "kind": "webhook" },
            "action": { "kind": "none" }
        })
        .to_string(),
    );
    assert_eq!(r.status(), 200, "seeding automation failed");
}

fn run_count(d: &Daemon) -> usize {
    let v: Value = call(ureq::get(&d.url("/automation-runs")))
        .into_json()
        .unwrap();
    v.as_array().map_or(0, Vec::len)
}

// 8. The hook token is never read from the query string.
#[test]
fn hook_query_token_is_ignored() {
    let d = boot_loopback("ag-8", PathBuf::new(), "h00k", "");
    seed_webhook_automation(&d);
    let runs = run_count(&d);
    let body = r#"{"hello":"world"}"#;
    let r = send(ureq::post(&d.url("/hooks/demo?token=h00k")), body);
    assert_eq!(r.status(), 401);
    let v: Value = r.into_json().unwrap();
    assert_eq!(
        v,
        json!({ "error": "unauthorized", "code": "unauthorized" })
    );
    assert_eq!(run_count(&d), runs, "a refused hook must not start a run");

    let ok = send(
        ureq::post(&d.url("/hooks/demo")).set("Authorization", "Bearer h00k"),
        body,
    );
    assert_ne!(ok.status(), 401);
    d.handle.stop();
}

// 9. GitHub signature: wrong is 401, right over the raw bytes is accepted.
#[test]
fn hook_signature_is_checked() {
    let d = boot_loopback("ag-9", PathBuf::new(), "h00k", "");
    seed_webhook_automation(&d);
    let body = r#"{"ref":"refs/heads/master"}"#;
    let bad = send(
        ureq::post(&d.url("/hooks/demo")).set("X-Hub-Signature-256", "sha256=00"),
        body,
    );
    assert_eq!(bad.status(), 401);
    let sig = format!("sha256={}", hmac_sha256_hex(b"h00k", body.as_bytes()));
    let good = send(
        ureq::post(&d.url("/hooks/demo")).set("X-Hub-Signature-256", &sig),
        body,
    );
    assert_ne!(good.status(), 401);
    d.handle.stop();
}

/// Open `GET /api/events?es=<t>` on a raw socket; return the status line and,
/// on 200, the first body line.
fn open_sse(d: &Daemon, es: &str) -> (String, Option<String>) {
    let mut s = TcpStream::connect(&d.host).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        s,
        "GET /api/events?es={es} HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\n\r\n",
        d.host
    )
    .unwrap();
    let mut rd = BufReader::new(s);
    let mut status = String::new();
    rd.read_line(&mut status).unwrap();
    if !status.contains(" 200") {
        return (status, None);
    }
    let mut line = String::new();
    loop {
        line.clear();
        if rd.read_line(&mut line).unwrap() == 0 {
            return (status, None);
        }
        if line == "\r\n" {
            break;
        }
    }
    // Chunked bodies prefix each chunk with its hex size; skip that line.
    loop {
        line.clear();
        if rd.read_line(&mut line).unwrap() == 0 {
            return (status, None);
        }
        let t = line.trim_end();
        if t.is_empty() || t.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        return (status, Some(t.to_string()));
    }
}

// 10. SSE tokens are minted with a header token and consumed once.
#[test]
fn sse_token_is_single_use() {
    let Some(d) = boot_remote("ag-10", "", "s3cr3t") else {
        return;
    };
    let v: Value = send(
        ureq::post(&d.url("/events/token")).set("X-Aidp-Token", "s3cr3t"),
        "",
    )
    .into_json()
    .unwrap();
    let t = v["token"].as_str().expect("token field").to_string();

    let (status, first) = open_sse(&d, &t);
    assert!(status.contains(" 200"), "first open: {status}");
    let first = first.expect("a first SSE line");
    assert!(first.starts_with(':'), "first SSE line was {first:?}");

    let (again, _) = open_sse(&d, &t);
    assert!(again.contains(" 401"), "second open: {again}");
    d.handle.stop();
}
