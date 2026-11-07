//! HTTP input hardening (LOC-25): end-to-end scenarios 5 to 11 of the ticket,
//! against a daemon really started through `serve_on` (the LOC-23 harness).
//!
//! Scenario numbering follows the ticket: 5 (body over the bound, 413, both
//! via `Read::take` and via `Content-Length`, plus the exact-limit body), 6/7/10
//! (memory stays bounded for a 413 or a 403 on a 400 MiB body), 8 (malformed
//! JSON is named), 9 (an empty body is unchanged), 11 (a non-ASCII request line
//! is closed by tiny_http). Scenarios 1 to 4 live in `http_contract.rs` and in
//! the unit tests of `src/http.rs`.
//!
//! Two deliberate harness choices:
//! - The client for big bodies is a raw `TcpStream` streaming from ONE fixed
//!   64 KiB buffer: the test process never allocates a body, so what is
//!   measured is the daemon, and a daemon closing early shows up as a write
//!   error (a test failure: LOC-25 drains the whole body before replying).
//! - Memory is measured on a daemon living in its own process (this test
//!   binary re-executed on its `memory::daemon_child` test): `PeakWorkingSet64` is a
//!   per-process high-water mark, and the other tests of this binary run
//!   concurrently in the same process.
//!
//! No test sets `AIDP_MAX_BODY_BYTES`: the environment is process-global and
//! the tests of a binary run in parallel threads, so the default
//! `MAX_BODY_BYTES` is the known limit.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle, MAX_BODY_BYTES};
use aidp_testkit::TempWorkspace;
use serde_json::Value;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Size of the single buffer every big body is streamed from.
const CHUNK: usize = 64 * 1024;

/// In-process daemon on an ephemeral port, same pattern as `http_contract.rs`.
fn boot_server(read_only: bool) -> (ServeHandle, u16) {
    let workspace = Box::leak(Box::new(TempWorkspace::new("gw-hardening")));
    let state_dir = workspace.path().to_path_buf();
    // Prevents the AutomationEngine from seeding its defaults.
    fs::write(state_dir.join(".defaults-seeded"), "").expect("failed to write .defaults-seeded");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
    let port = listener.local_addr().expect("no local_addr").port();

    let config = HttpConfig {
        state_dir,
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only,
        allowed_origins: vec![],
    };
    let handle = serve_on(listener, config).expect("serve_on failed");
    (handle, port)
}

fn make_url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{}{}", port, path)
}

/// How the request body is framed on the wire.
enum Framing {
    Length(u64),
    Chunked,
}

fn connect(port: u16) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to the daemon");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("set_read_timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(60)))
        .expect("set_write_timeout");
    stream
}

fn write_or_fail(stream: &mut TcpStream, bytes: &[u8]) {
    stream.write_all(bytes).unwrap_or_else(|e| {
        panic!(
            "write failed ({:?}: {e}): the daemon closed the connection before draining the \
             body, which LOC-25 rejects",
            e.kind()
        )
    });
}

fn write_head(stream: &mut TcpStream, method: &str, path: &str, framing: &Framing) {
    let framing_header = match framing {
        Framing::Length(n) => format!("Content-Length: {n}"),
        Framing::Chunked => "Transfer-Encoding: chunked".to_string(),
    };
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         {framing_header}\r\nConnection: close\r\n\r\n"
    );
    write_or_fail(stream, head.as_bytes());
}

/// Write `len` copies of `byte`, from one fixed buffer, as raw body bytes.
fn stream_fill(stream: &mut TcpStream, byte: u8, len: u64) {
    let buf = vec![byte; CHUNK];
    let mut left = len;
    while left > 0 {
        let n = left.min(CHUNK as u64) as usize;
        write_or_fail(stream, &buf[..n]);
        left -= n as u64;
    }
}

/// Write `len` copies of `byte` as a chunked body, terminator included.
fn stream_chunked(stream: &mut TcpStream, byte: u8, len: u64) {
    let buf = vec![byte; CHUNK];
    let mut left = len;
    while left > 0 {
        let n = left.min(CHUNK as u64) as usize;
        write_or_fail(stream, format!("{n:x}\r\n").as_bytes());
        write_or_fail(stream, &buf[..n]);
        write_or_fail(stream, b"\r\n");
        left -= n as u64;
    }
    write_or_fail(stream, b"0\r\n\r\n");
}

/// Status code and body of the whole response on `stream`.
fn raw_response(stream: &mut TcpStream) -> (u16, String) {
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .expect("failed to read the daemon's response");
    let text = String::from_utf8(raw).expect("response is not UTF-8");
    let status_line = text.lines().next().unwrap_or_default();
    let status = status_line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("no status in response {text:?}"));
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

/// `method path` with a body of `len` bytes of `byte`, then the response.
fn send_filled(
    port: u16,
    method: &str,
    path: &str,
    framing: Framing,
    byte: u8,
    len: u64,
) -> (u16, String) {
    let mut stream = connect(port);
    write_head(&mut stream, method, path, &framing);
    match framing {
        Framing::Length(_) => stream_fill(&mut stream, byte, len),
        Framing::Chunked => stream_chunked(&mut stream, byte, len),
    }
    raw_response(&mut stream)
}

fn assert_json_error(status: u16, body: &str, expected_status: u16) {
    assert_eq!(status, expected_status, "unexpected status, body {body:?}");
    let v: Value = serde_json::from_str(body)
        .unwrap_or_else(|e| panic!("response body is not JSON ({e}): {body:?}"));
    assert!(
        v["error"].is_string(),
        "response has no string `error` key: {body:?}"
    );
}

fn assert_no_session(port: u16) {
    let resp = ureq::get(&make_url(port, "/api/sessions"))
        .call()
        .expect("GET /api/sessions failed");
    assert_eq!(resp.status(), 200);
    let sessions: Value = resp.into_json().expect("invalid JSON response");
    assert_eq!(
        sessions.as_array().map(Vec::len),
        Some(0),
        "no session may be created by a refused body: {sessions}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 5: a body above the bound is refused with 413
// ─────────────────────────────────────────────────────────────────────────────

/// Chunked, no Content-Length: only the `Read::take` bound can catch it.
#[test]
fn scenario_5_body_over_limit_is_413_via_take() {
    let (handle, port) = boot_server(false);
    let (status, body) = send_filled(
        port,
        "POST",
        "/api/sessions",
        Framing::Chunked,
        b'a',
        MAX_BODY_BYTES as u64 + 1,
    );
    assert_json_error(status, &body, 413);
    assert_no_session(port);
    handle.stop();
}

/// Content-Length above the bound: refused on the header, before reading.
#[test]
fn scenario_5_body_over_limit_is_413_via_content_length() {
    let (handle, port) = boot_server(false);
    let len = MAX_BODY_BYTES as u64 + 1;
    let (status, body) = send_filled(
        port,
        "POST",
        "/api/sessions",
        Framing::Length(len),
        b'a',
        len,
    );
    assert_json_error(status, &body, 413);
    assert_no_session(port);
    handle.stop();
}

/// A valid body of exactly `MAX_BODY_BYTES` reaches the route: the bound is
/// `>`, not `>=`. The unknown project makes `SessionManager::start` answer
/// 404 without starting anything.
#[test]
fn scenario_5_body_at_limit_is_not_413() {
    let (handle, port) = boot_server(false);
    let prefix = br#"{"projectId":"no-such-project","prompt":""#;
    let suffix = br#""}"#;
    let fill = (MAX_BODY_BYTES - prefix.len() - suffix.len()) as u64;

    let mut stream = connect(port);
    write_head(
        &mut stream,
        "POST",
        "/api/sessions",
        &Framing::Length(MAX_BODY_BYTES as u64),
    );
    write_or_fail(&mut stream, prefix);
    stream_fill(&mut stream, b'a', fill);
    write_or_fail(&mut stream, suffix);
    let (status, body) = raw_response(&mut stream);

    assert_eq!(status, 404, "body {body:?}");
    assert_eq!(body, r#"{"code":"not_found","error":"project not found"}"#);
    assert_no_session(port);
    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 8: a malformed JSON body is named as such
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn scenario_8_invalid_json_is_named() {
    let (handle, port) = boot_server(false);
    {
        let resp = ureq::post(&make_url(port, "/api/sessions"))
            .set("Content-Type", "application/json")
            .send_string(r#"{"projectId":"abc","prompt":"hi"#)
            .unwrap_or_else(|e| e.into_response().unwrap());
        assert_eq!(resp.status(), 400);
        assert_eq!(
            resp.into_string().expect("response body"),
            r#"{"code":"bad_request","error":"invalid JSON body"}"#
        );
    }
    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 9: an empty body keeps today's answer
// ─────────────────────────────────────────────────────────────────────────────

/// FROZEN: an absent body is still `{}` and fails `StartRequest` validation;
/// the field is `projectId` because `StartRequest` is camelCase.
#[test]
fn scenario_9_empty_body_is_unchanged() {
    let (handle, port) = boot_server(false);
    {
        let resp = ureq::post(&make_url(port, "/api/sessions"))
            .send_string("")
            .unwrap_or_else(|e| e.into_response().unwrap());
        assert_eq!(resp.status(), 400);
        assert_eq!(
            resp.into_string().expect("response body"),
            r#"{"code":"bad_request","error":"invalid body: missing field `projectId`"}"#
        );
    }
    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 11: a non-ASCII request line is closed by tiny_http
// ─────────────────────────────────────────────────────────────────────────────

/// FROZEN: tiny_http (`client.rs:80-94`, `AsciiString::from_ascii`) rejects
/// the request line and closes the connection; `handle()` never runs. This is
/// dependency behaviour, only characterised here — the `urldecode` fix itself
/// is proven by the unit tests in `src/http.rs`.
#[test]
fn scenario_11_non_ascii_request_line_is_closed_by_tiny_http() {
    let (handle, port) = boot_server(false);
    {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set_read_timeout");
        write_or_fail(
            &mut stream,
            "GET /api/gateway/status?x=%a\u{e9} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".as_bytes(),
        );
        let mut buf = [0u8; 1024];
        match stream.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => panic!(
                "expected no response byte, got {n}: {:?}",
                String::from_utf8_lossy(&buf[..n])
            ),
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::UnexpectedEof
                ) => {}
            Err(e) => panic!(
                "expected the connection to be closed, got {:?}: {e}",
                e.kind()
            ),
        }
    }
    {
        let resp = ureq::get(&make_url(port, "/api/gateway/status"))
            .call()
            .expect("GET /api/gateway/status after the rejected line");
        assert_eq!(resp.status(), 200);
    }
    handle.stop();
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenarios 6, 7, 10: memory stays bounded (Windows only)
// ─────────────────────────────────────────────────────────────────────────────

/// The ticket's reference measure is `PeakWorkingSet64`, a Windows counter.
/// On other OSes these scenarios are compiled out; porting them means reading
/// `VmHWM` from `/proc/<pid>/status` instead.
#[cfg(windows)]
mod memory {
    use super::{send_filled, Framing};
    use aidp_gateway::http::{serve_on, HttpConfig};
    use aidp_testkit::TempWorkspace;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    const MIB: u64 = 1024 * 1024;
    /// The ticket's measured input.
    const BIG_BODY: usize = 400 * 1024 * 1024;
    const STATE_DIR_ENV: &str = "AIDP_HARDENING_CHILD_STATE_DIR";
    const READ_ONLY_ENV: &str = "AIDP_HARDENING_CHILD_READ_ONLY";

    /// Re-execution entry point: hosts one daemon in a process of its own.
    /// A no-op in a normal run, where `AIDP_HARDENING_CHILD_STATE_DIR` is unset.
    #[test]
    fn daemon_child() {
        let Some(state_dir) = std::env::var_os(STATE_DIR_ENV) else {
            return;
        };
        let state_dir = PathBuf::from(state_dir);
        std::fs::create_dir_all(&state_dir).expect("create the child state dir");
        std::fs::write(state_dir.join(".defaults-seeded"), "")
            .expect("failed to write .defaults-seeded");
        let read_only = std::env::var(READ_ONLY_ENV).ok().as_deref() == Some("1");

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to bind on port 0");
        let handle = serve_on(
            listener,
            HttpConfig {
                state_dir,
                vault_root: PathBuf::new(),
                hook_token: "hook-secret".to_string(),
                ui_token: "ui-secret".to_string(),
                read_only,
                allowed_origins: vec![],
            },
        )
        .expect("serve_on failed");
        println!("PORT={}", handle.port());
        std::io::stdout().flush().expect("flush stdout");
        // Serve until the parent closes our stdin (or kills us).
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        handle.stop();
    }

    /// `daemon_child`'s libtest name: the module path minus the hosting binary's crate.
    fn daemon_child_test_name() -> String {
        let module = module_path!()
            .split_once("::")
            .map_or(module_path!(), |(_, rest)| rest);
        format!("{module}::daemon_child")
    }

    /// A daemon in a child process, killed on drop.
    struct ChildDaemon {
        child: Child,
        port: u16,
        _ws: TempWorkspace,
    }

    impl ChildDaemon {
        fn spawn(read_only: bool) -> Self {
            let ws = TempWorkspace::new("gw-hardening-child");
            let mut child = Command::new(std::env::current_exe().expect("current_exe"))
                .args([
                    daemon_child_test_name().as_str(),
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(STATE_DIR_ENV, ws.path())
                .env(READ_ONLY_ENV, if read_only { "1" } else { "0" })
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("spawn the daemon child");

            // Read stdout on a thread so the 10 s deadline holds even if the
            // child never prints; the thread keeps draining until EOF.
            let stdout = child.stdout.take().expect("child stdout");
            let (tx, rx) = mpsc::channel::<String>();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    // Once the port is known nobody listens: keep draining.
                    let _ = tx.send(line);
                }
            });

            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut seen = Vec::new();
            let port = loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                match rx.recv_timeout(left) {
                    Ok(line) => {
                        // libtest prints `test <module>::daemon_child ... ` with
                        // no newline first, so the marker is mid-line.
                        if let Some((_, p)) = line.split_once("PORT=") {
                            break p.trim().parse::<u16>().expect("child port");
                        }
                        seen.push(line);
                    }
                    Err(e) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!(
                            "daemon child printed no PORT= line ({e:?}); stdout so far: {seen:?}"
                        );
                    }
                }
            };
            ChildDaemon {
                child,
                port,
                _ws: ws,
            }
        }

        fn pid(&self) -> u32 {
            self.child.id()
        }
    }

    impl Drop for ChildDaemon {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn peak_working_set_bytes(pid: u32) -> u64 {
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
            ])
            .output()
            .expect("run powershell Get-Process");
        let stdout = String::from_utf8_lossy(&out.stdout);
        stdout.trim().parse::<u64>().unwrap_or_else(|_| {
            panic!(
                "cannot read PeakWorkingSet64 of pid {pid}: stdout {:?}, stderr {:?}",
                stdout,
                String::from_utf8_lossy(&out.stderr)
            )
        })
    }

    fn assert_fresh_baseline(pid: u32) {
        let baseline = peak_working_set_bytes(pid);
        assert!(
            baseline < 32 * MIB,
            "baseline peak {baseline} bytes >= 32 MiB: the child is a fresh test binary hosting \
             one daemon, so the harness itself grew"
        );
    }

    fn assert_bounded_peak(pid: u32, before_fix: &str) {
        let after = peak_working_set_bytes(pid);
        assert!(
            after < 64 * MIB,
            "daemon peak working set {after} bytes ({:.1} MiB) >= 64 MiB after a {BIG_BODY}-byte \
             body; before LOC-25 the same request peaked at {before_fix}",
            after as f64 / MIB as f64
        );
    }

    #[test]
    fn scenario_6_413_bounds_daemon_memory() {
        let daemon = ChildDaemon::spawn(false);
        assert_fresh_baseline(daemon.pid());
        let (status, body) = send_filled(
            daemon.port,
            "POST",
            "/api/sessions",
            Framing::Length(BIG_BODY as u64),
            0,
            BIG_BODY as u64,
        );
        super::assert_json_error(status, &body, 413);
        assert_bounded_peak(daemon.pid(), "811.6 MB");
    }

    #[test]
    fn scenario_7_get_with_huge_content_length_is_413_and_bounded() {
        let daemon = ChildDaemon::spawn(false);
        assert_fresh_baseline(daemon.pid());
        let (status, body) = send_filled(
            daemon.port,
            "GET",
            "/api/gateway/status",
            Framing::Length(419_430_400),
            0,
            419_430_400,
        );
        // Before LOC-25: 200 OK, and tiny_http's EqualReader::drop allocated it all.
        super::assert_json_error(status, &body, 413);
        assert_bounded_peak(daemon.pid(), "409.8 MB (and a 200 OK)");
    }

    #[test]
    fn scenario_10_read_only_403_does_not_buffer() {
        let daemon = ChildDaemon::spawn(true);
        assert_fresh_baseline(daemon.pid());
        let (status, body) = send_filled(
            daemon.port,
            "POST",
            "/api/sessions",
            Framing::Length(BIG_BODY as u64),
            0,
            BIG_BODY as u64,
        );
        assert_eq!(status, 403, "body {body:?}");
        assert_eq!(
            body,
            r#"{"code":"forbidden","error":"read-only mode: set AIDP_READ_ONLY=0 to allow writes"}"#
        );
        assert_bounded_peak(daemon.pid(), "811.6 MB");
    }
}
