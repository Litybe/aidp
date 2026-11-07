//! Daemon concurrency against a really-started gateway and a fake `claude`:
//! admission is not held across the first stdin write, cancel stays responsive,
//! a stalled SSE subscriber stays bounded, shutdown is clean, and requests
//! beyond the in-flight ceiling get `503`.

use aidp_gateway::bus::{Bus, SUBSCRIBER_CAPACITY};
use aidp_gateway::http::{
    serve_on, HttpConfig, ServeHandle, SSE_OVERFLOW_EVENT, TOO_MANY_REQUESTS_MESSAGE,
};
use aidp_testkit::{pid_alive, EnvGuard, FakeClaude, FakeClaudeConfig, TempWorkspace};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Environment variable naming the `claude` executable the daemon spawns.
const CLAUDE_BIN_ENV: &str = "AIDP_CLAUDE_BIN";
/// Environment variable overriding the in-flight request ceiling.
const MAX_INFLIGHT_ENV: &str = "AIDP_MAX_INFLIGHT_REQUESTS";
/// Marker file that stops the automation engine from seeding defaults.
const DEFAULTS_SEEDED_MARKER: &str = ".defaults-seeded";
/// Directory of the daemon state holding one JSON record per session.
const SESSIONS_DIR: &str = "sessions";
/// Hook token of the test daemon.
const HOOK_TOKEN: &str = "hook-secret";
/// UI token of the test daemon (not required from loopback).
const UI_TOKEN: &str = "ui-secret";
/// Loopback address every test daemon binds to.
const LOOPBACK: &str = "127.0.0.1";

/// Engine that sends the raw prompt to `claude`.
const ENGINE_PROMPT: &str = "prompt";
/// Session status after `POST /api/sessions/:id/cancel`.
const STATUS_CANCELLED: &str = "cancelled";
/// Session status of a live session found by a daemon shutdown.
const STATUS_ORPHANED: &str = "orphaned";
/// Status of the sessions seeded before boot.
const STATUS_DONE: &str = "done";
/// Short prompt of an ordinary session start.
const SHORT_PROMPT: &str = "hello";
/// Character repeated to build the large prompt.
const LARGE_PROMPT_FILLER: char = 'x';

/// Comment frame an SSE stream sends first.
const SSE_CONNECTED_MARKER: &str = ": connected";
/// Prefix of an SSE data line.
const SSE_DATA_PREFIX: &str = "data: ";
/// Status line prefix of a successful raw HTTP response.
const HTTP_OK_PREFIX: &str = "HTTP/1.1 200";
/// Last bytes of a chunked body.
const CHUNKED_TERMINATOR: &[u8] = b"0\r\n\r\n";

/// Timeout of one ordinary test HTTP call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Read timeout of the SSE client that keeps reading.
const SSE_READ_TIMEOUT: Duration = Duration::from_secs(60);
/// Read timeout of one raw socket read.
const RAW_READ_TIMEOUT: Duration = Duration::from_millis(500);
/// Interval between two polls of a condition.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Deadline for a fake `claude` to record its PID.
const PID_DEADLINE: Duration = Duration::from_secs(10);
/// Deadline for a killed process to disappear or a status to settle.
const SETTLE_DEADLINE: Duration = Duration::from_secs(10);
/// Deadline for an SSE stream to send its connect frame.
const CONNECT_DEADLINE: Duration = Duration::from_secs(10);
/// Deadline for the listener port to be bindable again after `stop()`.
const REBIND_DEADLINE: Duration = Duration::from_secs(1);
/// Upper bound on a responsive call (start, cancel, `stop()`).
/// The guarded bugs block 30 s and up, so 10 s tolerates a loaded machine.
const RESPONSIVE_BOUND: Duration = Duration::from_secs(10);

/// Filler bytes the fake writes to stdout before reading its prompt.
const ADMISSION_STDOUT_BYTES: usize = 256 * 1024;
/// Size of the prompt written to the first session's stdin.
const LARGE_PROMPT_BYTES: usize = 1024 * 1024;
/// Seconds the fake sleeps in the admission scenario.
const ADMISSION_SLEEP_SECS: u64 = 30;
/// Seconds the fake sleeps in the cancel scenario.
const CANCEL_SLEEP_SECS: u64 = 60;
/// Seconds the fake sleeps in the shutdown scenario.
const SHUTDOWN_SLEEP_SECS: u64 = 300;
/// Delay between the first and second session start.
const SECOND_START_DELAY: Duration = Duration::from_secs(2);

/// Events emitted before overflow is checked.
const MIN_OVERFLOW_EVENTS: usize = 10_000;
/// Hard cap on emitted events while waiting for overflow.
const MAX_OVERFLOW_EVENTS: usize = 200_000;
/// Events emitted per flood round, split across the workers.
const EVENTS_PER_ROUND: usize = 1_000;
/// Parallel workers mutating one seeded session each.
const FLOOD_WORKERS: usize = 4;
/// Deadline of the whole flood-until-overflow loop.
const OVERFLOW_DEADLINE: Duration = Duration::from_secs(180);
/// Deadline for the stalled stream to be read to its end.
const DRAIN_DEADLINE: Duration = Duration::from_secs(60);
/// Size of one raw socket read.
const RAW_READ_CHUNK: usize = 64 * 1024;
/// Events emitted straight on a bus with a subscriber that never reads.
const BUS_EMITS: usize = 10_000;

/// In-flight ceiling of the saturation test.
const SATURATION_CEILING: usize = 4;
/// Status code of a request refused at the ceiling.
const SERVICE_UNAVAILABLE: u16 = 503;
/// Status code of a successful request.
const OK: u16 = 200;

/// A running daemon, stopped when dropped so a failed assertion kills its children.
struct Daemon {
    handle: Option<ServeHandle>,
    port: u16,
    state_dir: PathBuf,
}

impl Daemon {
    fn url(&self, path: &str) -> String {
        format!("http://{LOOPBACK}:{}{path}", self.port)
    }

    fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A temp workspace that lives until the test process exits.
fn leaked_workspace(tag: &str) -> &'static TempWorkspace {
    Box::leak(Box::new(TempWorkspace::new(tag)))
}

/// Boot a daemon on port 0; `seed` writes into the state dir before `serve_on`.
fn boot_daemon(tag: &str, seed: impl FnOnce(&Path)) -> Daemon {
    let state_dir = leaked_workspace(tag).path().to_path_buf();
    fs::write(state_dir.join(DEFAULTS_SEEDED_MARKER), "").expect("write defaults marker");
    seed(&state_dir);
    let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind port 0");
    let port = listener.local_addr().expect("local addr").port();
    let config = HttpConfig {
        state_dir: state_dir.clone(),
        vault_root: PathBuf::new(),
        hook_token: HOOK_TOKEN.to_string(),
        ui_token: UI_TOKEN.to_string(),
        read_only: false,
        allowed_origins: vec![],
    };
    let handle = serve_on(listener, config).expect("serve_on failed");
    Daemon {
        handle: Some(handle),
        port,
        state_dir,
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build()
}

/// Turn a ureq result into (status, JSON body), whatever the status.
fn status_and_body(result: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
    let response = result.unwrap_or_else(|e| e.into_response().expect("transport error"));
    let status = response.status();
    (status, response.into_json().unwrap_or(Value::Null))
}

fn post_json(daemon: &Daemon, path: &str, body: Value) -> (u16, Value) {
    status_and_body(agent().post(&daemon.url(path)).send_json(body))
}

fn get_json(daemon: &Daemon, path: &str) -> (u16, Value) {
    status_and_body(agent().get(&daemon.url(path)).call())
}

/// Poll `condition` until it holds; panics naming `what` once `deadline` elapses.
fn wait_until(deadline: Duration, what: &str, mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + deadline;
    while !condition() {
        assert!(Instant::now() < end, "timed out after {deadline:?}: {what}");
        thread::sleep(POLL_INTERVAL);
    }
}

/// Register a fresh project directory named `name`; returns its id.
fn register_project(daemon: &Daemon, name: &str) -> String {
    let dir = daemon.state_dir.join(name);
    fs::create_dir_all(&dir).expect("create project dir");
    let (status, body) = post_json(
        daemon,
        "/api/projects",
        json!({ "path": dir.to_string_lossy() }),
    );
    assert_eq!(status, OK, "project registration failed: {body}");
    body["id"].as_str().expect("project id").to_string()
}

fn start_request(project_id: &str, prompt: &str) -> Value {
    json!({ "projectId": project_id, "engine": ENGINE_PROMPT, "prompt": prompt })
}

/// Start a session and assert it answered `200` with an id; returns the id.
fn start_session(daemon: &Daemon, project_id: &str, prompt: &str) -> String {
    let (status, body) = post_json(daemon, "/api/sessions", start_request(project_id, prompt));
    session_id_of(status, &body)
}

fn session_id_of(status: u16, body: &Value) -> String {
    assert_eq!(status, OK, "session start failed: {body}");
    body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no session id in {body}"))
        .to_string()
}

fn cancel_session(daemon: &Daemon, id: &str) -> u16 {
    post_json(daemon, &format!("/api/sessions/{id}/cancel"), json!({})).0
}

fn session_status(daemon: &Daemon, id: &str) -> Option<String> {
    let (status, body) = get_json(daemon, &format!("/api/sessions/{id}"));
    (status == OK)
        .then(|| body["status"].as_str().map(str::to_string))
        .flatten()
}

/// Wait until `fake` recorded at least `count` launches; returns their PIDs.
fn wait_for_pids(fake: &FakeClaude, count: usize) -> Vec<u32> {
    wait_until(PID_DEADLINE, "fake claude recording its PID", || {
        fake.pids().len() >= count
    });
    fake.pids()
}

fn wait_until_dead(pid: u32) {
    wait_until(SETTLE_DEADLINE, &format!("process {pid} exiting"), || {
        !pid_alive(pid)
    });
}

fn install_fake(tag: &str, config: &FakeClaudeConfig) -> FakeClaude {
    FakeClaude::install(leaked_workspace(tag), "claude", config)
}

fn claude_bin_guard(fake: &FakeClaude) -> EnvGuard {
    let bin = fake
        .exe_path()
        .to_str()
        .expect("utf-8 fake path")
        .to_string();
    EnvGuard::with(&[(CLAUDE_BIN_ENV, Some(&bin)), (MAX_INFLIGHT_ENV, None)])
}

#[test]
fn scenario_2_admission_not_held_across_stdin_write() {
    let fake = install_fake(
        "gw-conc-admit-bin",
        &FakeClaudeConfig {
            stdout_bytes: ADMISSION_STDOUT_BYTES,
            read_stdin_line: true,
            sleep_secs: ADMISSION_SLEEP_SECS,
            ..FakeClaudeConfig::default()
        },
    );
    let _env = claude_bin_guard(&fake);
    let daemon = boot_daemon("gw-conc-admit", |_| {});
    let project_a = register_project(&daemon, "project-a");
    let project_b = register_project(&daemon, "project-b");

    let large_prompt: String =
        std::iter::repeat_n(LARGE_PROMPT_FILLER, LARGE_PROMPT_BYTES).collect();
    let url_a = daemon.url("/api/sessions");
    let body_a = start_request(&project_a, &large_prompt);
    let start_a = thread::spawn(move || status_and_body(agent().post(&url_a).send_json(body_a)));
    thread::sleep(SECOND_START_DELAY);

    let started = Instant::now();
    let (status_b, body_b) = post_json(
        &daemon,
        "/api/sessions",
        start_request(&project_b, SHORT_PROMPT),
    );
    let elapsed_b = started.elapsed();
    let id_b = session_id_of(status_b, &body_b);
    assert!(
        elapsed_b < RESPONSIVE_BOUND,
        "second start took {elapsed_b:?} while the first prompt was being written"
    );

    let (status_a, body_a) = start_a.join().expect("first start thread panicked");
    let id_a = session_id_of(status_a, &body_a);

    for id in [&id_a, &id_b] {
        assert_eq!(cancel_session(&daemon, id), OK, "cancel {id}");
    }
}

#[test]
fn scenario_3_cancel_is_responsive() {
    let fake = install_fake(
        "gw-conc-cancel-bin",
        &FakeClaudeConfig {
            sleep_secs: CANCEL_SLEEP_SECS,
            ..FakeClaudeConfig::default()
        },
    );
    let _env = claude_bin_guard(&fake);
    let daemon = boot_daemon("gw-conc-cancel", |_| {});
    let project = register_project(&daemon, "project");
    let id = start_session(&daemon, &project, SHORT_PROMPT);
    let pid = wait_for_pids(&fake, 1)[0];

    assert_cancel_is_responsive(&daemon, &id, pid);
}

/// Cancel `id` within the responsive bound, then check its status and process.
fn assert_cancel_is_responsive(daemon: &Daemon, id: &str, pid: u32) {
    let started = Instant::now();
    let status = cancel_session(daemon, id);
    let elapsed = started.elapsed();
    assert_eq!(status, OK, "cancel answered {status}");
    assert!(elapsed < RESPONSIVE_BOUND, "cancel took {elapsed:?}");
    wait_until(SETTLE_DEADLINE, "session status cancelled", || {
        session_status(daemon, id).as_deref() == Some(STATUS_CANCELLED)
    });
    wait_until_dead(pid);
}

/// A fake that closes its stdout before sleeping, so the daemon's stdout reader is
/// already waiting for process exit when cancel arrives. Windows cannot close stdout
/// without unsafe code, so there the lock fix is proven by a unit test in sessions.rs.
#[cfg(unix)]
#[test]
fn scenario_3_cancel_is_responsive_with_closed_stdout() {
    use std::os::unix::fs::PermissionsExt;
    const SCRIPT_MODE: u32 = 0o755;
    let dir = leaked_workspace("gw-conc-cancel-sh-bin")
        .path()
        .to_path_buf();
    let pid_file = dir.join("claude.pids");
    let script = dir.join("claude.sh");
    let body = format!(
        "#!/bin/sh\necho $$ > '{}'\nexec 1>&-\nexec sleep {CANCEL_SLEEP_SECS}\n",
        pid_file.display()
    );
    fs::write(&script, body).expect("write fake script");
    fs::set_permissions(&script, fs::Permissions::from_mode(SCRIPT_MODE)).expect("chmod script");

    let bin = script.to_str().expect("utf-8 script path").to_string();
    let _env = EnvGuard::with(&[(CLAUDE_BIN_ENV, Some(&bin)), (MAX_INFLIGHT_ENV, None)]);
    let daemon = boot_daemon("gw-conc-cancel-sh", |_| {});
    let project = register_project(&daemon, "project");
    let id = start_session(&daemon, &project, SHORT_PROMPT);
    let read_pid = || {
        fs::read_to_string(&pid_file)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
    };
    wait_until(PID_DEADLINE, "script recording its PID", || {
        read_pid().is_some()
    });
    let pid = read_pid().expect("script PID");

    assert_cancel_is_responsive(&daemon, &id, pid);
}

/// Seed one finished session record per flood worker; returns their ids.
fn seed_sessions(state_dir: &Path) -> Vec<String> {
    let sessions_dir = state_dir.join(SESSIONS_DIR);
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    (0..FLOOD_WORKERS)
        .map(|worker| {
            let id = format!("seeded-{worker}");
            let record = json!({
                "id": id,
                "projectId": "seeded-project",
                "projectName": "Seeded project",
                "cwd": state_dir.to_string_lossy(),
                "engine": ENGINE_PROMPT,
                "prompt": SHORT_PROMPT,
                "permissionMode": "default",
                "status": STATUS_DONE,
                "startedAt": "2024-01-01T00:00:00.000Z",
                "origin": { "kind": "api" },
            });
            fs::write(sessions_dir.join(format!("{id}.json")), record.to_string())
                .expect("write seeded session");
            id
        })
        .collect()
}

/// Open `GET /api/events` on a raw socket and read up to its connect frame.
fn open_raw_event_stream(port: u16) -> TcpStream {
    let mut stream = TcpStream::connect((LOOPBACK, port)).expect("connect raw SSE");
    stream
        .set_read_timeout(Some(RAW_READ_TIMEOUT))
        .expect("set read timeout");
    write!(
        stream,
        "GET /api/events HTTP/1.1\r\nHost: {LOOPBACK}:{port}\r\nConnection: close\r\n\r\n"
    )
    .expect("write raw SSE request");
    let received = read_until_marker(&mut stream, SSE_CONNECTED_MARKER);
    assert!(
        received.starts_with(HTTP_OK_PREFIX),
        "raw SSE did not answer 200: {received}"
    );
    stream
}

/// Read `stream` until `marker` shows up; returns what was read.
fn read_until_marker(stream: &mut TcpStream, marker: &str) -> String {
    let end = Instant::now() + CONNECT_DEADLINE;
    let mut received = Vec::new();
    let mut chunk = [0u8; RAW_READ_CHUNK];
    loop {
        let text = String::from_utf8_lossy(&received).to_string();
        if text.contains(marker) {
            return text;
        }
        assert!(
            Instant::now() < end,
            "no {marker:?} within {CONNECT_DEADLINE:?}: {text}"
        );
        match stream.read(&mut chunk) {
            Ok(0) => panic!("stream closed before {marker:?}: {text}"),
            Ok(n) => received.extend_from_slice(&chunk[..n]),
            Err(e) if is_timeout(&e) => {}
            Err(e) => panic!("read failed before {marker:?}: {e}"),
        }
    }
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Read `stream` to its end; true once the daemon closed it or ended the chunked body.
fn drains_to_end(stream: &mut TcpStream) -> bool {
    let end = Instant::now() + DRAIN_DEADLINE;
    let mut chunk = vec![0u8; RAW_READ_CHUNK];
    let mut tail: Vec<u8> = Vec::new();
    while Instant::now() < end {
        match stream.read(&mut chunk) {
            Ok(0) => return true,
            Ok(n) => {
                tail.extend_from_slice(&chunk[..n]);
                let keep = tail.len().saturating_sub(CHUNKED_TERMINATOR.len());
                tail.drain(..keep);
                if tail == CHUNKED_TERMINATOR {
                    return true;
                }
            }
            Err(e) if is_timeout(&e) => {}
            Err(_) => return true,
        }
    }
    false
}

/// What the SSE client that keeps reading has seen so far.
#[derive(Default)]
struct EventLog {
    connected: AtomicBool,
    overflow_seen: AtomicBool,
    received: AtomicUsize,
}

/// Follow `GET /api/events` on a thread, recording the connect frame and overflow.
fn spawn_event_reader(port: u16) -> Arc<EventLog> {
    let log = Arc::new(EventLog::default());
    let thread_log = Arc::clone(&log);
    let url = format!("http://{LOOPBACK}:{port}/api/events");
    thread::spawn(move || {
        let reader_agent = ureq::AgentBuilder::new()
            .timeout_connect(REQUEST_TIMEOUT)
            .timeout_read(SSE_READ_TIMEOUT)
            .build();
        let Ok(response) = reader_agent.get(&url).call() else {
            return;
        };
        for line in BufReader::new(response.into_reader()).lines() {
            let Ok(line) = line else {
                return;
            };
            record_sse_line(&thread_log, &line);
            if thread_log.overflow_seen.load(Ordering::SeqCst) {
                return;
            }
        }
    });
    log
}

fn record_sse_line(log: &EventLog, line: &str) {
    if line == SSE_CONNECTED_MARKER {
        log.connected.store(true, Ordering::SeqCst);
    }
    let Some(payload) = line.strip_prefix(SSE_DATA_PREFIX) else {
        return;
    };
    let Ok(event) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    log.received.fetch_add(1, Ordering::SeqCst);
    if event["type"].as_str() == Some(SSE_OVERFLOW_EVENT) {
        log.overflow_seen.store(true, Ordering::SeqCst);
    }
}

/// Emit `per_worker` bus events per seeded session, in parallel; returns how many landed.
fn flood_events(daemon: &Daemon, session_ids: &[String], per_worker: usize) -> usize {
    let emitted = AtomicUsize::new(0);
    thread::scope(|scope| {
        for id in session_ids {
            let url = daemon.url(&format!("/api/sessions/{id}"));
            let emitted = &emitted;
            scope.spawn(move || {
                let worker_agent = agent();
                for round in 0..per_worker {
                    let body = json!({ "title": format!("title {round}") });
                    let (status, _) =
                        status_and_body(worker_agent.request("PATCH", &url).send_json(body));
                    if status == OK {
                        emitted.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
        }
    });
    emitted.into_inner()
}

#[test]
fn scenario_5_stalled_sse_subscriber_stays_bounded() {
    let _env = EnvGuard::unset(MAX_INFLIGHT_ENV);
    let mut session_ids = Vec::new();
    let daemon = boot_daemon("gw-conc-sse", |state_dir| {
        session_ids = seed_sessions(state_dir);
    });

    let mut stalled = open_raw_event_stream(daemon.port);
    let log = spawn_event_reader(daemon.port);
    wait_until(CONNECT_DEADLINE, "reading SSE client connected", || {
        log.connected.load(Ordering::SeqCst)
    });

    let per_worker = EVENTS_PER_ROUND / FLOOD_WORKERS;
    let end = Instant::now() + OVERFLOW_DEADLINE;
    let mut emitted = 0;
    while emitted < MAX_OVERFLOW_EVENTS && Instant::now() < end {
        if emitted >= MIN_OVERFLOW_EVENTS && log.overflow_seen.load(Ordering::SeqCst) {
            break;
        }
        emitted += flood_events(&daemon, &session_ids, per_worker);
    }

    assert!(
        emitted >= MIN_OVERFLOW_EVENTS,
        "only {emitted} events emitted"
    );
    assert!(
        log.overflow_seen.load(Ordering::SeqCst),
        "no {SSE_OVERFLOW_EVENT:?} event seen after {emitted} events ({} received)",
        log.received.load(Ordering::SeqCst)
    );
    assert!(
        drains_to_end(&mut stalled),
        "stalled SSE stream was not closed within {DRAIN_DEADLINE:?}"
    );
}

#[test]
fn scenario_5_bus_subscriber_queue_is_bounded() {
    let bus = Bus::new();
    let never_read = bus.subscribe();
    for index in 0..BUS_EMITS {
        bus.emit("probe", json!({ "index": index }));
    }
    let queued = never_read.try_iter().count();
    assert!(
        queued <= SUBSCRIBER_CAPACITY,
        "{queued} events queued for one subscriber, bound is {SUBSCRIBER_CAPACITY}"
    );
    assert_eq!(bus.dropped_events(), (BUS_EMITS - queued) as u64);
}

fn session_file_status(state_dir: &Path, id: &str) -> Option<String> {
    let path = state_dir.join(SESSIONS_DIR).join(format!("{id}.json"));
    let record: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    record["status"].as_str().map(str::to_string)
}

#[test]
fn scenario_6_clean_shutdown() {
    let fake = install_fake(
        "gw-conc-stop-bin",
        &FakeClaudeConfig {
            sleep_secs: SHUTDOWN_SLEEP_SECS,
            ..FakeClaudeConfig::default()
        },
    );
    let _env = claude_bin_guard(&fake);
    let mut daemon = boot_daemon("gw-conc-stop", |_| {});
    let ids: Vec<String> = ["project-a", "project-b"]
        .iter()
        .map(|name| {
            let project = register_project(&daemon, name);
            start_session(&daemon, &project, SHORT_PROMPT)
        })
        .collect();
    let pids = wait_for_pids(&fake, ids.len());

    let port = daemon.port;
    let started = Instant::now();
    daemon.stop();
    let elapsed = started.elapsed();
    assert!(elapsed < RESPONSIVE_BOUND, "stop() took {elapsed:?}");

    wait_until(REBIND_DEADLINE, "listener port rebindable", || {
        TcpListener::bind((LOOPBACK, port)).is_ok()
    });
    for pid in pids {
        wait_until_dead(pid);
    }
    for id in &ids {
        assert_eq!(
            session_file_status(&daemon.state_dir, id).as_deref(),
            Some(STATUS_ORPHANED),
            "session {id} not orphaned on disk"
        );
    }
}

#[test]
fn scenario_7_request_beyond_ceiling_gets_503() {
    let ceiling = SATURATION_CEILING.to_string();
    let _env = EnvGuard::set(MAX_INFLIGHT_ENV, &ceiling);
    let daemon = boot_daemon("gw-conc-ceiling", |_| {});

    let open_streams: Vec<TcpStream> = (0..SATURATION_CEILING)
        .map(|_| open_raw_event_stream(daemon.port))
        .collect();

    let (status, body) = get_json(&daemon, "/api/health");
    assert_eq!(
        status, SERVICE_UNAVAILABLE,
        "expected 503, got {status}: {body}"
    );
    assert_eq!(body["error"].as_str(), Some(TOO_MANY_REQUESTS_MESSAGE));
    drop(open_streams);
}
