//! Background-work behaviour of a really-started daemon: the health probe cache,
//! a stable process cwd during a CI probe, scheduler re-entrancy and the local
//! dispatcher tick. Every scenario holds an `EnvGuard`, so they run one at a time.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_testkit::{pid_alive, EnvGuard, FakeClaude, FakeClaudeConfig, TempWorkspace};
use serde_json::{json, Value};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Loopback host every server of this file binds to.
const LOOPBACK: &str = "127.0.0.1";
/// Marker file that stops the automation engine from seeding its defaults.
const DEFAULTS_SEEDED_MARKER: &str = ".defaults-seeded";
/// Token the daemon expects on `/api/hooks`.
const HOOK_TOKEN: &str = "hook-secret";
/// Token the daemon expects from off-loopback callers.
const UI_TOKEN: &str = "ui-secret";
/// Connect timeout of the test's own HTTP client.
const CLIENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Read timeout of the test's own HTTP client; covers a cold health probe.
const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(120);
/// Pause between two polls of a condition.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Pause between two accept attempts on a non-blocking stub listener.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Health route.
const HEALTH_PATH: &str = "/api/health";
/// Recent bus events route, asking for the whole buffer.
const RECENT_EVENTS_PATH: &str = "/api/events/recent?limit=500";
/// Automation creation route.
const AUTOMATIONS_PATH: &str = "/api/automations";
/// Key of the thread snapshot in the health body.
const THREADS_KEY: &str = "threads";

/// Stem the fake is installed under, so `PATH` resolves it as `claude`.
const CLAUDE_STEM: &str = "claude";
/// Environment variable naming the claude executable.
const CLAUDE_BIN_VAR: &str = "AIDP_CLAUDE_BIN";
/// Process search path variable.
const PATH_VAR: &str = "PATH";
/// Seconds the fake sleeps: far longer than the daemon's probe timeout.
const FAKE_CLAUDE_SLEEP_SECS: u64 = 30;
/// Longest a health call may take once the probe cache is warm.
const WARM_HEALTH_BUDGET: Duration = Duration::from_secs(1);
/// Wait for the daemon's probe cache to expire, when an earlier scenario warmed it.
const PROBE_CACHE_EXPIRY_WAIT: Duration = Duration::from_secs(32);
/// Longest wait for a killed probe process to disappear.
const PROBE_KILL_DEADLINE: Duration = Duration::from_secs(10);

/// Environment variables that keep git and gh from prompting.
const NON_INTERACTIVE_ENV: &[(&str, Option<&str>)] = &[
    (GIT_TERMINAL_PROMPT_VAR, Some(DISABLED_FLAG)),
    (GH_PROMPT_DISABLED_VAR, Some(ENABLED_FLAG)),
];
/// Git variable disabling credential prompts.
const GIT_TERMINAL_PROMPT_VAR: &str = "GIT_TERMINAL_PROMPT";
/// gh variable disabling interactive prompts.
const GH_PROMPT_DISABLED_VAR: &str = "GH_PROMPT_DISABLED";
/// Value turning a boolean environment flag off.
const DISABLED_FLAG: &str = "0";
/// Value turning a boolean environment flag on.
const ENABLED_FLAG: &str = "1";

/// Trigger kind of a cron automation.
const TRIGGER_KIND_CRON: &str = "cron";
/// Shortest schedule the daemon's cron grammar allows: once a minute.
const EVERY_MINUTE_CRON: &str = "* * * * *";
/// Action kind posting to a URL.
const ACTION_KIND_WEBHOOK: &str = "webhook";
/// Method of the webhook action.
const WEBHOOK_METHOD: &str = "POST";
/// Path the webhook action posts to.
const WEBHOOK_PATH: &str = "/hook";
/// Bus event emitted when a due automation is still running.
const AUTOMATION_SKIPPED_EVENT: &str = "automation.skipped";
/// Reason carried by that event.
const SKIPPED_ALREADY_RUNNING: &str = "already running";
/// Longest wait for a skipped fire: two minute boundaries plus scheduler slack.
const SKIP_EVENT_DEADLINE: Duration = Duration::from_secs(150);
/// Pause between two response bytes of the slow stub, kept under the daemon's read timeout.
const TRICKLE_INTERVAL: Duration = Duration::from_secs(2);
/// Status line the slow stub opens its never-ending response with.
const TRICKLE_PREAMBLE: &[u8] = b"HTTP/1.1 200 OK\r\nX-Slow: ";
/// One byte of the never-ending header value.
const TRICKLE_BYTE: &[u8] = b"a";
/// Most webhook connections ever allowed open at once.
const MAX_CONCURRENT_WEBHOOKS: usize = 1;

/// A daemon served on an ephemeral port, with the workspace holding its state.
struct Daemon {
    handle: ServeHandle,
    port: u16,
    _workspace: TempWorkspace,
}

impl Daemon {
    /// Boots a daemon with no vault.
    fn boot() -> Daemon {
        Daemon::boot_with_vault(PathBuf::new())
    }

    /// Boots a daemon whose dispatcher reads the vault at `vault_root`.
    fn boot_with_vault(vault_root: PathBuf) -> Daemon {
        let workspace = TempWorkspace::new("gw-concurrency");
        let state_dir = workspace.path().to_path_buf();
        fs::write(state_dir.join(DEFAULTS_SEEDED_MARKER), "").expect("write defaults marker");
        let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind an ephemeral port");
        let config = HttpConfig {
            state_dir,
            vault_root,
            hook_token: HOOK_TOKEN.to_string(),
            ui_token: UI_TOKEN.to_string(),
            read_only: false,
            allowed_origins: vec![],
        };
        let handle = serve_on(listener, config).expect("serve_on failed");
        let port = handle.port();
        Daemon {
            handle,
            port,
            _workspace: workspace,
        }
    }

    /// Sends one request and returns its status and JSON body, error statuses included.
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let url = format!("http://{LOOPBACK}:{}{path}", self.port);
        let request = client().request(method, &url);
        let outcome = match body {
            Some(payload) => request.send_json(payload),
            None => request.call(),
        };
        let response = match outcome {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(error) => panic!("{method} {path} failed: {error}"),
        };
        let status = response.status();
        let json_body: Value = response.into_json().unwrap_or(Value::Null);
        (status, json_body)
    }

    /// `GET path`.
    fn get(&self, path: &str) -> (u16, Value) {
        self.call("GET", path, None)
    }

    /// `POST path` with a JSON body.
    fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.call("POST", path, Some(body))
    }

    /// Stops the daemon, then drops its workspace.
    fn stop(self) {
        self.handle.stop();
    }
}

/// The test's own HTTP client, with explicit timeouts.
fn client() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CLIENT_CONNECT_TIMEOUT)
        .timeout_read(CLIENT_READ_TIMEOUT)
        .build()
}

/// Polls `condition` until it holds or `deadline` elapses; returns whether it held.
fn wait_until(deadline: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// `PATH` with `dir` searched first.
fn path_with_first(dir: &Path) -> String {
    let current = std::env::var_os(PATH_VAR).unwrap_or_default();
    let entries = std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&current));
    let joined = std::env::join_paths(entries).expect("join PATH entries");
    joined.into_string().expect("PATH is valid unicode")
}

/// Connection counters of a stub server.
#[derive(Default)]
struct ConnectionStats {
    accepted: AtomicUsize,
    open: AtomicUsize,
    max_open: AtomicUsize,
}

/// A TCP stub that accepts connections and never completes an answer.
struct StubServer {
    port: u16,
    stats: Arc<ConnectionStats>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl StubServer {
    /// Slow stub: opens a response and trickles one header byte per interval, forever.
    fn trickling() -> StubServer {
        StubServer::start(true)
    }

    fn start(trickle: bool) -> StubServer {
        let listener = TcpListener::bind((LOOPBACK, 0)).expect("bind the stub");
        listener.set_nonblocking(true).expect("non-blocking stub");
        let port = listener.local_addr().expect("stub address").port();
        let stats = Arc::new(ConnectionStats::default());
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));
        let accept_thread = {
            let (stats, stop, connections) = (stats.clone(), stop.clone(), connections.clone());
            thread::spawn(move || accept_loop(listener, trickle, stats, stop, connections))
        };
        StubServer {
            port,
            stats,
            stop,
            accept_thread: Some(accept_thread),
            connections,
        }
    }

    fn base_url(&self) -> String {
        format!("http://{LOOPBACK}:{}", self.port)
    }

    fn accepted(&self) -> usize {
        self.stats.accepted.load(Ordering::SeqCst)
    }

    fn max_open(&self) -> usize {
        self.stats.max_open.load(Ordering::SeqCst)
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(accept) = self.accept_thread.take() {
            let _ = accept.join();
        }
        let handles: Vec<JoinHandle<()>> = self
            .connections
            .lock()
            .map(|mut list| list.drain(..).collect())
            .unwrap_or_default();
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// Accepts until stopped, serving each connection on its own thread.
fn accept_loop(
    listener: TcpListener,
    trickle: bool,
    stats: Arc<ConnectionStats>,
    stop: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                stats.accepted.fetch_add(1, Ordering::SeqCst);
                let open_now = stats.open.fetch_add(1, Ordering::SeqCst) + 1;
                stats.max_open.fetch_max(open_now, Ordering::SeqCst);
                let (stats, stop) = (stats.clone(), stop.clone());
                let handle = thread::spawn(move || {
                    hold_connection(stream, trickle, &stop);
                    stats.open.fetch_sub(1, Ordering::SeqCst);
                });
                if let Ok(mut list) = connections.lock() {
                    list.push(handle);
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL_INTERVAL)
            }
            Err(_) => thread::sleep(ACCEPT_POLL_INTERVAL),
        }
    }
}

/// Holds one connection until the peer closes it or the stub stops.
fn hold_connection(mut stream: TcpStream, trickle: bool, stop: &AtomicBool) {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(TRICKLE_INTERVAL)).is_err()
    {
        return;
    }
    if trickle && stream.write_all(TRICKLE_PREAMBLE).is_err() {
        return;
    }
    let mut buffer = [0u8; 4096];
    while !stop.load(Ordering::SeqCst) {
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if is_read_timeout(&error) => {
                if trickle && stream.write_all(TRICKLE_BYTE).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

/// Whether a read error only means the read timeout elapsed.
fn is_read_timeout(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Whether the recent bus events hold a skipped fire of `automation_id`.
fn has_skipped_event(events: &Value, automation_id: &str) -> bool {
    events.as_array().is_some_and(|list| {
        list.iter().any(|event| {
            event["type"] == AUTOMATION_SKIPPED_EVENT
                && event["data"]["automationId"] == automation_id
                && event["data"]["skipped"] == SKIPPED_ALREADY_RUNNING
        })
    })
}

#[test]
fn scenario_8_health_probe_is_cached_and_leaves_no_process() {
    let workspace = TempWorkspace::new("gw-fake-claude");
    let config = FakeClaudeConfig {
        sleep_secs: FAKE_CLAUDE_SLEEP_SECS,
        ..FakeClaudeConfig::default()
    };
    let fake = FakeClaude::install(&workspace, CLAUDE_STEM, &config);
    let search_path = path_with_first(fake.dir());
    let fake_exe = fake.exe_path().display().to_string();
    let _env = EnvGuard::with(&[
        (PATH_VAR, Some(search_path.as_str())),
        (CLAUDE_BIN_VAR, Some(fake_exe.as_str())),
    ]);
    let daemon = Daemon::boot();

    let (status, first) = daemon.get(HEALTH_PATH);
    let (status, first) = if fake.pids().is_empty() {
        eprintln!("probe cache warmed by an earlier scenario; waiting for it to expire");
        thread::sleep(PROBE_CACHE_EXPIRY_WAIT);
        daemon.get(HEALTH_PATH)
    } else {
        (status, first)
    };
    assert_eq!(status, 200, "first health call: {first}");
    assert!(first[THREADS_KEY].is_object(), "no threads object: {first}");
    let pids = fake.pids();
    assert!(
        !pids.is_empty(),
        "the cold health probe never launched the fake claude"
    );

    let started = Instant::now();
    let (status, second) = daemon.get(HEALTH_PATH);
    let warm_elapsed = started.elapsed();
    assert_eq!(status, 200, "second health call: {second}");
    assert!(
        warm_elapsed < WARM_HEALTH_BUDGET,
        "warm health call took {warm_elapsed:?}"
    );
    assert!(
        second[THREADS_KEY].is_object(),
        "no threads object: {second}"
    );

    let all_dead = wait_until(PROBE_KILL_DEADLINE, || {
        pids.iter().all(|pid| !pid_alive(*pid))
    });
    assert!(all_dead, "a fake claude outlived its probe: {pids:?}");
    daemon.stop();
}

#[test]
#[ignore = "waits for two real cron minute boundaries (~2 min); CI runs it with --ignored"]
fn scenario_10_scheduler_never_overlaps_one_automation() {
    let _env = EnvGuard::with(NON_INTERACTIVE_ENV);
    let stub = StubServer::trickling();
    let daemon = Daemon::boot();
    let webhook_url = format!("{}{WEBHOOK_PATH}", stub.base_url());
    let (status, automation) = daemon.post(
        AUTOMATIONS_PATH,
        json!({
            "name": "slow webhook",
            "trigger": { "kind": TRIGGER_KIND_CRON, "expr": EVERY_MINUTE_CRON },
            "action": { "kind": ACTION_KIND_WEBHOOK, "url": webhook_url, "method": WEBHOOK_METHOD },
        }),
    );
    assert_eq!(status, 200, "automation creation: {automation}");
    let automation_id = automation["id"]
        .as_str()
        .expect("automation id")
        .to_string();

    let skipped = wait_until(SKIP_EVENT_DEADLINE, || {
        let (status, events) = daemon.get(RECENT_EVENTS_PATH);
        status == 200 && has_skipped_event(&events, &automation_id)
    });

    assert!(stub.accepted() >= 1, "the webhook never reached the stub");
    assert!(
        stub.max_open() <= MAX_CONCURRENT_WEBHOOKS,
        "{} webhook connections were open at once",
        stub.max_open()
    );
    assert!(
        skipped,
        "no `{AUTOMATION_SKIPPED_EVENT}` / `{SKIPPED_ALREADY_RUNNING}` event for {automation_id}"
    );
    daemon.stop();
    drop(stub);
}
