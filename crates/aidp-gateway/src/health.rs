//! Data source behind `GET /api/health`.
//! Doctor is obtained by shelling out to the gateway's own binary, degrading gracefully.

use crate::models::doctor_summary::{
    DoctorSummary, CURRENT_EXE_UNAVAILABLE, DOCTOR_OUTPUT_UNPARSEABLE, DOCTOR_PROBE_PANICKED,
    DOCTOR_SPAWN_FAILED,
};
use crate::models::enums::{SessionEventKind, SessionStatus};
use crate::models::ProviderHealth;
use crate::sessions::SessionManager;
use crate::sync::lock;
use aidp_agents::{PROVIDERS_KEY, PROVIDER_VERSION_ARG};
use aidp_core::clock::{Clock, SystemClock};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long we allow `claude auth status` to run before giving up on the
/// probe and falling back to the turn-event heuristic.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How often `wait_or_kill` checks whether a probe child has exited.
const PROBE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How long a doctor or login probe result is reused before it is run again.
const PROBE_CACHE_TTL: Duration = Duration::from_secs(30);

/// Key of the resident-loop snapshot in the `GET /api/health` body.
const THREADS_KEY: &str = "threads";

/// Key of the worktrees whose deletion keeps failing in the `GET /api/health` body.
const WORKTREES_REMOVE_FAILED_KEY: &str = "worktreesRemoveFailed";

/// Key of the purged session ids in the `POST /api/health/cleanup` body.
const CLEANUP_PURGED_KEY: &str = "purged";

/// Key of the purged session count in the `POST /api/health/cleanup` body.
const CLEANUP_COUNT_KEY: &str = "count";

/// Key of the worktree sweep result in the `POST /api/health/cleanup` body.
const CLEANUP_WORKTREES_KEY: &str = "worktrees";

/// Key of the `aidp` binary report in the `GET /api/health` body.
const AIDP_BINARY_KEY: &str = "aidpBinary";

/// Key carrying the failure reason in a degraded `aidpBinary` block.
const AIDP_BINARY_ERROR_KEY: &str = "error";

/// The `aidp` binary probe panicked while running on its own thread.
const AIDP_BINARY_PROBE_PANICKED: &str = "aidp binary probe panicked";

/// How long a provider `--version` probe may run.
const PROVIDER_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Program run to probe the Claude login.
const CLAUDE_PROGRAM: &str = "claude";

/// Arguments of the Claude login probe.
const CLAUDE_AUTH_STATUS_ARGS: [&str; 2] = ["auth", "status"];

/// Arguments of the doctor probe.
const DOCTOR_ARGS: [&str; 3] = ["doctor", "--format", "json-compact"];

/// Doctor envelope key holding the checks array.
const CHECKS_KEY: &str = "checks";

/// Doctor envelope key holding a boolean verdict.
const OK_KEY: &str = "ok";

/// Doctor check key holding the check name.
const NAME_KEY: &str = "name";

/// Doctor check name reported when hooks are missing.
const HOOKS_INSTALLED_CHECK: &str = "hooks_installed";

/// Subtype an authentication error result carries.
const AUTH_ERROR_SUBTYPE: &str = "error_authentication";

/// Session event key flagging an error.
const EVENT_IS_ERROR_KEY: &str = "isError";

/// Session event key naming its kind.
const EVENT_KIND_KEY: &str = "kind";

/// Session event key carrying its result text.
const EVENT_RESULT_KEY: &str = "result";

/// Session event key carrying its subtype.
const EVENT_SUBTYPE_KEY: &str = "subtype";

/// Key of the login state in the login block.
const LOGIN_STATE_KEY: &str = "state";

/// Claude login state reported to clients.
#[derive(Clone, Copy)]
enum LoginState {
    Expired,
    Ok,
    Unknown,
}

impl LoginState {
    /// The wire value of this state.
    const fn as_str(self) -> &'static str {
        match self {
            LoginState::Expired => "expired",
            LoginState::Ok => "ok",
            LoginState::Unknown => "unknown",
        }
    }
}

/// Where a login state was derived from.
#[derive(Clone, Copy)]
enum LoginSource {
    Probe,
    TurnEnd,
    None,
}

impl LoginSource {
    /// The wire value of this source.
    const fn as_str(self) -> &'static str {
        match self {
            LoginSource::Probe => "probe",
            LoginSource::TurnEnd => "turn_end",
            LoginSource::None => "none",
        }
    }
}

/// Success flag, stdout and stderr of a probe that exited before its timeout.
type ProbeOutcome = Option<(bool, String, String)>;

/// A cached value with the instant it was computed, empty until first use,
/// plus the mutex that makes a concurrent refresh single-flight (see `cached`).
struct CacheSlot<T> {
    value: Mutex<Option<(Instant, T)>>,
    refresh: Mutex<()>,
}

impl<T> CacheSlot<T> {
    fn empty() -> Self {
        CacheSlot {
            value: Mutex::new(None),
            refresh: Mutex::new(()),
        }
    }
}

/// Lowercase markers of an expired/absent-login error result, matched
/// case-insensitively by `looks_like_auth_error`.
const AUTH_ERROR_MARKERS: &[&str] = &[
    "invalid api key",
    "please run /login",
    "/login",
    "oauth token has expired",
    "authentication_error",
    "unauthorized",
    "not logged in",
];

/// Does `text` or `subtype` look like an expired/absent Claude login?
/// Conservative: ordinary errors and non-auth subtypes return `false`.
pub fn looks_like_auth_error(text: &str, subtype: Option<&str>) -> bool {
    if let Some(st) = subtype {
        if st == AUTH_ERROR_SUBTYPE || st.to_lowercase().contains("auth") {
            return true;
        }
    }
    let lower = text.to_lowercase();
    if AUTH_ERROR_MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    lower.contains("credentials") && lower.contains("expired")
}

/// Newest `turn_end` event with `isError:true` that looks like an expired
/// login, as a trimmed excerpt of at most 200 chars.
fn expired_from_events(events: &[Value]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|e| {
            e.get(EVENT_KIND_KEY).and_then(Value::as_str)
                == Some(SessionEventKind::TurnEnd.as_str())
                && e.get(EVENT_IS_ERROR_KEY).and_then(Value::as_bool) == Some(true)
        })
        .and_then(|e| {
            let result = e
                .get(EVENT_RESULT_KEY)
                .and_then(Value::as_str)
                .unwrap_or("");
            let subtype = e.get(EVENT_SUBTYPE_KEY).and_then(Value::as_str);
            if looks_like_auth_error(result, subtype) {
                let mut excerpt: String = result.trim().chars().take(200).collect();
                if result.trim().chars().count() > 200 {
                    excerpt.push('\u{2026}');
                }
                Some(excerpt)
            } else {
                None
            }
        })
}

/// Run a `Command` bounded by `timeout`; `None` on spawn failure or timeout,
/// killing and reaping the child.
fn run_bounded_with(mut cmd: Command, timeout: Duration) -> ProbeOutcome {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout_reader = spawn_pipe_reader(child.stdout.take());
    let stderr_reader = spawn_pipe_reader(child.stderr.take());
    let status = wait_or_kill(&mut child, timeout)?;
    Some((
        status.success(),
        join_pipe_reader(stdout_reader),
        join_pipe_reader(stderr_reader),
    ))
}

/// Drain one child pipe on its own thread, so a full pipe buffer can never
/// stall the child while `wait_or_kill` polls it.
fn spawn_pipe_reader<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut captured);
        }
        captured
    })
}

/// Collect what a pipe reader captured, lossily decoded as UTF-8.
fn join_pipe_reader(reader: JoinHandle<Vec<u8>>) -> String {
    let captured = reader.join().unwrap_or_default();
    String::from_utf8_lossy(&captured).into_owned()
}

/// Poll `child` until it exits or `timeout` elapses; on expiry or a read
/// failure it is killed and reaped, returning `None`.
fn wait_or_kill(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(PROBE_POLL_INTERVAL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The value in `slot` if it is younger than `ttl`, cloned under the value
/// lock alone — never held while a probe runs.
fn fresh_value<T: Clone>(slot: &CacheSlot<T>, ttl: Duration) -> Option<T> {
    lock(&slot.value)
        .as_ref()
        .filter(|(computed_at, _)| computed_at.elapsed() < ttl)
        .map(|(_, value)| value.clone())
}

/// Whatever value is in `slot`, fresh or stale, or `None` on a cold cache.
fn stale_value<T: Clone>(slot: &CacheSlot<T>) -> Option<T> {
    lock(&slot.value).as_ref().map(|(_, value)| value.clone())
}

/// `mutex.try_lock()`, recovering a poisoned mutex the same way `lock()` does
/// rather than propagating the poison.
fn try_refresh_lock(mutex: &Mutex<()>) -> Option<MutexGuard<'_, ()>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

/// Re-checks freshness under the refresh lock, then runs `compute` with no
/// lock held and stores the result.
fn refresh_and_store<T: Clone>(
    slot: &CacheSlot<T>,
    ttl: Duration,
    compute: impl FnOnce() -> T,
) -> T {
    if let Some(value) = fresh_value(slot, ttl) {
        return value;
    }
    let value = compute();
    *lock(&slot.value) = Some((Instant::now(), value.clone()));
    value
}

/// Single-flight cache read: one caller refreshes a cold or expired slot.
/// Concurrent misses wait (cold) or get the stale value (warm).
fn cached<T: Clone>(slot: &CacheSlot<T>, ttl: Duration, compute: impl FnOnce() -> T) -> T {
    if let Some(value) = fresh_value(slot, ttl) {
        return value;
    }
    match try_refresh_lock(&slot.refresh) {
        Some(_guard) => refresh_and_store(slot, ttl, compute),
        None => stale_value(slot).unwrap_or_else(|| {
            let _guard = slot.refresh.lock().unwrap_or_else(|e| e.into_inner());
            refresh_and_store(slot, ttl, compute)
        }),
    }
}

/// A worktree the dispatcher could not delete, reported by `GET /api/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRemoveFailure {
    pub key: String,
    pub path: String,
    pub error: String,
    pub attempts: u32,
}

/// A worktree deleted by a sweep, with the ref that kept its unsaved work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweptWorktree {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salvaged_ref: Option<String>,
}

/// What one worktree sweep deleted and what it could not delete.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorktreeSweepReport {
    pub removed: Vec<SweptWorktree>,
    pub failed: Vec<WorktreeRemoveFailure>,
}

/// Process-wide list of worktrees whose deletion failed, read by `health_json`.
/// The dispatcher writes it, so the health payload needs no handle on the dispatcher.
fn remove_failures_slot() -> &'static Mutex<Vec<WorktreeRemoveFailure>> {
    static SLOT: OnceLock<Mutex<Vec<WorktreeRemoveFailure>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Vec::new()))
}

/// Remember a failed deletion, replacing an earlier failure of the same ticket.
pub fn record_worktree_remove_failure(failure: WorktreeRemoveFailure) {
    let mut failures = lock(remove_failures_slot());
    failures.retain(|existing| existing.key != failure.key);
    failures.push(failure);
}

/// Forget the failed deletion of `key`, once the worktree is gone.
pub fn clear_worktree_remove_failure(key: &str) {
    lock(remove_failures_slot()).retain(|existing| existing.key != key);
}

/// The worktrees whose deletion currently fails.
pub fn worktree_remove_failures() -> Vec<WorktreeRemoveFailure> {
    lock(remove_failures_slot()).clone()
}

/// The `POST /api/health/cleanup` body: purged sessions and the worktree sweep.
pub fn cleanup_json(purged: &[String], sweep: &WorktreeSweepReport) -> Value {
    json!({
        OK_KEY: true,
        CLEANUP_COUNT_KEY: purged.len(),
        CLEANUP_PURGED_KEY: purged,
        CLEANUP_WORKTREES_KEY: sweep,
    })
}

/// Process-wide cache of the `aidp doctor` summary.
fn doctor_slot() -> &'static CacheSlot<Value> {
    static SLOT: OnceLock<CacheSlot<Value>> = OnceLock::new();
    SLOT.get_or_init(CacheSlot::empty)
}

/// Process-wide cache of the `claude auth status` probe outcome.
fn login_probe_slot() -> &'static CacheSlot<ProbeOutcome> {
    static SLOT: OnceLock<CacheSlot<ProbeOutcome>> = OnceLock::new();
    SLOT.get_or_init(CacheSlot::empty)
}

/// Process-wide cache of the `aidp` binary report.
fn aidp_binary_slot() -> &'static CacheSlot<Value> {
    static SLOT: OnceLock<CacheSlot<Value>> = OnceLock::new();
    SLOT.get_or_init(CacheSlot::empty)
}

/// The `aidp` binary report, reused for `PROBE_CACHE_TTL`.
fn cached_aidp_binary_json() -> Value {
    cached(aidp_binary_slot(), PROBE_CACHE_TTL, || {
        aidp_core::aidp_binary::probe().to_json()
    })
}

/// The `aidpBinary` block when the probe thread panicked: `ok` is null and
/// `error` names the reason, so the endpoint never answers a 500.
fn aidp_binary_unavailable() -> Value {
    json!({
        aidp_core::aidp_binary::KEY_OK: Value::Null,
        AIDP_BINARY_ERROR_KEY: AIDP_BINARY_PROBE_PANICKED,
    })
}

/// The probe thread's block, or the degraded block when it panicked.
fn aidp_binary_block(joined: std::thread::Result<Value>) -> Value {
    joined.unwrap_or_else(|_| aidp_binary_unavailable())
}

/// Process-wide cache of the agent provider report.
fn providers_slot() -> &'static CacheSlot<Value> {
    static SLOT: OnceLock<CacheSlot<Value>> = OnceLock::new();
    SLOT.get_or_init(CacheSlot::empty)
}

/// One `{id, program, found, version}` row per probed provider; `probe` returns the version.
pub fn providers_value(
    config: &aidp_agents::ProvidersConfig,
    probe: impl Fn(&str) -> Option<String>,
) -> Value {
    let rows: Vec<Value> = config
        .providers_to_probe()
        .into_iter()
        .filter_map(|(id, program)| {
            let version = probe(&program);
            let row = ProviderHealth {
                id,
                program,
                found: version.is_some(),
                version,
            };
            serde_json::to_value(row).ok()
        })
        .collect();
    Value::Array(rows)
}

/// First line of `<program> --version`, `None` when absent, failing or timed out.
fn probe_provider_version(program: &str) -> Option<String> {
    aidp_core::program::resolve_program(program)?;
    let mut cmd = Command::new(program);
    cmd.arg(PROVIDER_VERSION_ARG);
    let (success, stdout, _stderr) = run_bounded_with(cmd, PROVIDER_PROBE_TIMEOUT)?;
    success.then(|| stdout.lines().next().unwrap_or_default().trim().to_string())
}

/// The provider report read from the user config, reused for `PROBE_CACHE_TTL`.
fn cached_providers_json() -> Value {
    cached(providers_slot(), PROBE_CACHE_TTL, || {
        let config = aidp_core::vault::home_dir()
            .and_then(|home| aidp_agents::ProvidersConfig::load(&home).ok())
            .unwrap_or_default();
        providers_value(&config, probe_provider_version)
    })
}

/// The `aidp doctor` summary, reused for `PROBE_CACHE_TTL`.
fn cached_doctor_json() -> Value {
    cached(doctor_slot(), PROBE_CACHE_TTL, doctor_json)
}

/// The `claude auth status` probe outcome, reused for `PROBE_CACHE_TTL`.
fn cached_login_probe() -> ProbeOutcome {
    cached(login_probe_slot(), PROBE_CACHE_TTL, probe_claude_login)
}

/// Run `claude auth status`, bounded by `PROBE_TIMEOUT`.
fn probe_claude_login() -> ProbeOutcome {
    let mut cmd = Command::new(CLAUDE_PROGRAM);
    cmd.args(CLAUDE_AUTH_STATUS_ARGS);
    run_bounded_with(cmd, PROBE_TIMEOUT)
}

/// Run the doctor, login and `aidp` binary probes concurrently, so a cold
/// cache costs one `PROBE_TIMEOUT` rather than several.
fn probe_health_inputs() -> (Value, ProbeOutcome, Value) {
    std::thread::scope(|scope| {
        let doctor_handle = scope.spawn(cached_doctor_json);
        let aidp_binary_handle = scope.spawn(cached_aidp_binary_json);
        let login_probe = cached_login_probe();
        let doctor = doctor_handle
            .join()
            .unwrap_or_else(|_| DoctorSummary::unavailable(DOCTOR_PROBE_PANICKED).to_value());
        let aidp_binary = aidp_binary_block(aidp_binary_handle.join());
        (doctor, login_probe, aidp_binary)
    })
}

fn runtime_health_inputs(sessions: &SessionManager) -> (Value, ProbeOutcome, Value) {
    let environment = sessions.runtime_environment();
    let Some(program) = environment.get(crate::aidp_bin::AIDP_BIN_ENV) else {
        return probe_health_inputs();
    };
    let program = Path::new(program);
    std::thread::scope(|scope| {
        let doctor = scope.spawn(|| doctor_json_for_command(doctor_command(program, &environment)));
        let login_probe = cached_login_probe();
        let resolved = program.is_file().then(|| program.to_path_buf());
        let binary = aidp_core::aidp_binary::evaluate(aidp_core::aidp_binary::AidpBinaryFacts {
            resolved: resolved.clone(),
            running: resolved,
            ..Default::default()
        })
        .to_json();
        (
            doctor
                .join()
                .unwrap_or_else(|_| DoctorSummary::unavailable(DOCTOR_PROBE_PANICKED).to_value()),
            login_probe,
            binary,
        )
    })
}

fn doctor_command(
    program: &Path,
    environment: &std::collections::BTreeMap<String, String>,
) -> Command {
    let mut command = Command::new(program);
    command.args(DOCTOR_ARGS).envs(environment);
    command
}

/// Run `<current_exe> doctor` and parse its JSON envelope; only a spawn
/// failure or unparseable stdout is an error, the exit status is ignored.
fn doctor_json() -> Value {
    let Some(exe) = std::env::current_exe().ok() else {
        return DoctorSummary::unavailable(CURRENT_EXE_UNAVAILABLE).to_value();
    };
    doctor_json_for_command(doctor_command(&exe, &Default::default()))
}

fn doctor_json_for_command(cmd: Command) -> Value {
    let Some((_success, stdout, _stderr)) = run_bounded_with(cmd, PROBE_TIMEOUT) else {
        return DoctorSummary::unavailable(DOCTOR_SPAWN_FAILED).to_value();
    };
    let Ok(parsed) = serde_json::from_str::<Value>(stdout.trim()) else {
        return DoctorSummary::unavailable(DOCTOR_OUTPUT_UNPARSEABLE).to_value();
    };
    let checks = parsed
        .get(CHECKS_KEY)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let failing: Vec<String> = checks
        .iter()
        .filter(|c| c.get(OK_KEY).and_then(Value::as_bool) == Some(false))
        .filter_map(|c| c.get(NAME_KEY).and_then(Value::as_str).map(str::to_string))
        .collect();
    let ok = parsed.get(OK_KEY).and_then(Value::as_bool).unwrap_or(false);
    DoctorSummary {
        ok,
        checks: checks.len(),
        failing,
        error: None,
    }
    .to_value()
}

fn now_iso() -> String {
    SystemClock.now_utc().to_rfc3339()
}

/// Claude login state from the `claude auth status` probe; an inconclusive
/// probe falls back to the turn-event heuristic over recent sessions.
pub fn login_state(sessions: &SessionManager) -> Value {
    login_state_from(cached_login_probe(), sessions)
}

/// Turn a `claude auth status` probe outcome into the login state, falling
/// back to the turn-event heuristic when the probe is inconclusive.
fn login_state_from(login_probe: ProbeOutcome, sessions: &SessionManager) -> Value {
    if let Some((success, stdout, stderr)) = login_probe {
        let combined = format!("{stdout}\n{stderr}");
        if looks_like_auth_error(&combined, None) {
            return json!({
                LOGIN_STATE_KEY: LoginState::Expired.as_str(),
                "source": LoginSource::Probe.as_str(),
                "detail": combined.trim().chars().take(200).collect::<String>(),
                "checkedAt": now_iso(),
            });
        }
        if success {
            return json!({
                LOGIN_STATE_KEY: LoginState::Ok.as_str(),
                "source": LoginSource::Probe.as_str(),
                "detail": Value::Null,
                "checkedAt": now_iso(),
            });
        }
    }

    for rec in sessions.list(None, None, 20) {
        let events = sessions.events(&rec.id);
        if let Some(detail) = expired_from_events(&events) {
            return json!({
                LOGIN_STATE_KEY: LoginState::Expired.as_str(),
                "source": LoginSource::TurnEnd.as_str(),
                "detail": format!("{detail} (session {})", rec.id),
                "checkedAt": now_iso(),
            });
        }
    }

    json!({
        LOGIN_STATE_KEY: LoginState::Unknown.as_str(),
        "source": LoginSource::None.as_str(),
        "detail": Value::Null,
        "checkedAt": now_iso(),
    })
}

/// Assemble the `GET /api/health` JSON body.
pub fn health_json(state_dir: &Path, sessions: &SessionManager, hooks_enabled: bool) -> Value {
    let (doctor, login_probe, aidp_binary) = runtime_health_inputs(sessions);
    let doctor_ok = doctor.get(OK_KEY).and_then(Value::as_bool).unwrap_or(false);
    let failing = doctor.get("failing").and_then(Value::as_array);
    let hooks_installed = failing
        .map(|f| !f.iter().any(|v| v.as_str() == Some(HOOKS_INSTALLED_CHECK)))
        .unwrap_or(false);

    let login = login_state_from(login_probe, sessions);
    let login_expired =
        login.get(LOGIN_STATE_KEY).and_then(Value::as_str) == Some(LoginState::Expired.as_str());

    let (bytes, files) = crate::store::dir_size(state_dir);

    let running = sessions.running_count();
    let orphaned = sessions
        .list(Some(SessionStatus::Orphaned), None, 100_000)
        .len();

    json!({
        "ok": doctor_ok && hooks_installed && !login_expired,
        "doctor": doctor,
        "hooks": {
            "webhooksEnabled": hooks_enabled,
            "hooksInstalled": hooks_installed,
        },
        "claudeLogin": login,
        "stateDir": {
            "path": state_dir.display().to_string(),
            "bytes": bytes,
            "files": files,
        },
        "sessions": {
            "running": running,
            "orphaned": orphaned,
        },
        AIDP_BINARY_KEY: aidp_binary,
        PROVIDERS_KEY: cached_providers_json(),
        WORKTREES_REMOVE_FAILED_KEY: worktree_remove_failures(),
        THREADS_KEY: crate::threads::snapshot(),
    })
}

#[cfg(test)]
#[path = "../tests/unit/health_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/health_providers_tests.rs"]
mod health_providers_tests;
