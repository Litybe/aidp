//! `flow unity-refresh`: one compile gate (contract C1) of the Unity editor open on the workspace,
//! run right after `git.checkout` of a test flow so that the reimport and the recompilation of the
//! new branch end before the test stage starts, under control. The verb writes the request file of
//! the playtest package's file bridge, waits for its result and prints the verdict. A silent editor
//! is reported as `blocked` (with its last heartbeat), never as a crash of the verb.

use super::CommandOutput;
use crate::models::constants::flow_unity::*;
use crate::models::unity_refresh_report::UnityRefreshReport;
use aidp_gateway::native_runs::scope_guard::is_unity_project;
use aidp_gateway::unity_status::{find_editor, ProcessSource, SystemProcesses};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The wall clock and the editor processes the verb depends on; tests script both.
pub(super) struct Environment<'a> {
    pub processes: &'a dyn ProcessSource,
    pub timeout: Duration,
    pub poll: Duration,
    /// Times the heartbeat tolerance is stretched: 1 by default, more when the doctor granted a longer wait.
    pub wait_multiplier: i64,
    /// Brings the editor to the foreground before the request.
    pub focus: bool,
}

pub(super) fn unity_refresh(
    workspace: &Path,
    timeout_seconds: u64,
    wait_multiplier: u32,
) -> anyhow::Result<CommandOutput> {
    let multiplier = wait_multiplier.max(1);
    let environment = Environment {
        processes: &SystemProcesses,
        timeout: Duration::from_secs(timeout_seconds * u64::from(multiplier)),
        poll: Duration::from_millis(POLL_INTERVAL_MILLIS),
        wait_multiplier: i64::from(multiplier),
        focus: cfg!(windows),
    };
    output(&run(workspace, &environment))
}

/// Runs the gate and builds its report.
pub(super) fn run(workspace: &Path, environment: &Environment<'_>) -> UnityRefreshReport {
    if !is_unity_project(workspace) {
        return skipped(
            REASON_NOT_UNITY_PROJECT,
            "no Unity project in the workspace",
        );
    }
    let processes = environment.processes.unity_processes();
    if find_editor(&processes, &workspace.to_string_lossy()).is_none() {
        return skipped(
            REASON_EDITOR_CLOSED,
            "no Unity editor open on the workspace",
        );
    }

    let focused = environment
        .focus
        .then(|| super::unity_focus::focus_editor(workspace).focused);

    let request_id = request_id();
    if let Err(error) = write_request(workspace, &request_id) {
        return UnityRefreshReport {
            request_id: Some(request_id),
            blocked: true,
            focused,
            message: format!("{BLOCKED_UNITY_EDITOR}: cannot write the compile request: {error}"),
            ..UnityRefreshReport::default()
        };
    }

    let started = Instant::now();
    let result_path = result_file(workspace, &request_id);
    loop {
        if let Some(result) = read_json(&result_path) {
            return UnityRefreshReport {
                focused,
                ..from_result(request_id, &result)
            };
        }
        let heartbeat = read_json(&heartbeat_file(workspace)).unwrap_or(Value::Null);
        if let Some(age) =
            stale_heartbeat_age_scaled(&heartbeat, chrono::Utc::now(), environment.wait_multiplier)
        {
            return UnityRefreshReport {
                focused,
                ..blocked(
                    request_id,
                    format!(
                        "{BLOCKED_UNITY_EDITOR}: the editor heartbeat is {age} s old, its main thread is stuck (dialog box or freeze)"
                    ),
                    heartbeat,
                )
            };
        }
        if started.elapsed() >= environment.timeout {
            return UnityRefreshReport {
                focused,
                ..blocked(
                    request_id,
                    format!(
                        "{BLOCKED_UNITY_EDITOR}: no compile result after {} s",
                        environment.timeout.as_secs()
                    ),
                    heartbeat,
                )
            };
        }
        std::thread::sleep(environment.poll);
    }
}

fn skipped(reason: &str, message: &str) -> UnityRefreshReport {
    UnityRefreshReport {
        skipped: true,
        reason: Some(reason.to_string()),
        message: message.to_string(),
        ..UnityRefreshReport::default()
    }
}

fn blocked(request_id: String, message: String, heartbeat: Value) -> UnityRefreshReport {
    UnityRefreshReport {
        request_id: Some(request_id),
        blocked: true,
        message,
        heartbeat,
        ..UnityRefreshReport::default()
    }
}

/// The report of a compile result the package wrote (`ok`, `errors`, `warnings`, `durationMs`).
pub(super) fn from_result(request_id: String, result: &Value) -> UnityRefreshReport {
    let ok = result
        .get(RESULT_FIELD_OK)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let errors = result
        .get(RESULT_FIELD_ERRORS)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let message = if ok {
        "compile gate ok".to_string()
    } else {
        format!("compile gate failed: {} error(s)", errors.len())
    };
    UnityRefreshReport {
        request_id: Some(request_id),
        ok,
        errors,
        warnings: result
            .get(RESULT_FIELD_WARNINGS)
            .and_then(Value::as_u64)
            .unwrap_or(0),
        duration_ms: result
            .get(RESULT_FIELD_DURATION_MS)
            .and_then(Value::as_u64)
            .unwrap_or(0),
        message,
        ..UnityRefreshReport::default()
    }
}

/// Seconds the heartbeat has been silent when that silence means a stuck editor, limits times
/// `multiplier`. `None` for a live heartbeat or no heartbeat at all.
pub(super) fn stale_heartbeat_age_scaled(
    heartbeat: &Value,
    now: chrono::DateTime<chrono::Utc>,
    multiplier: i64,
) -> Option<i64> {
    let at = heartbeat.get(HEARTBEAT_FIELD_AT)?.as_str()?;
    let at = chrono::DateTime::parse_from_rfc3339(at).ok()?;
    let age = now.signed_duration_since(at).num_seconds();
    let reloading = heartbeat.get(HEARTBEAT_FIELD_STATE).and_then(Value::as_str)
        == Some(HEARTBEAT_STATE_RELOADING);
    let limit = if reloading {
        HEARTBEAT_RELOAD_STALE_SECONDS
    } else {
        HEARTBEAT_STALE_SECONDS
    };
    (age > limit * multiplier).then_some(age)
}

fn request_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    format!("{REQUEST_ID_PREFIX}{millis}")
}

fn dir(workspace: &Path, parts: &[&str; 3]) -> PathBuf {
    parts
        .iter()
        .fold(workspace.to_path_buf(), |path, part| path.join(part))
}

pub(super) fn request_file(workspace: &Path, request_id: &str) -> PathBuf {
    dir(workspace, &COMPILE_REQUESTS_DIR).join(format!("{request_id}.{JSON_EXTENSION}"))
}

pub(super) fn result_file(workspace: &Path, request_id: &str) -> PathBuf {
    dir(workspace, &COMPILE_RESULTS_DIR).join(format!("{request_id}.{JSON_EXTENSION}"))
}

pub(super) fn heartbeat_file(workspace: &Path) -> PathBuf {
    dir(workspace, &HEARTBEAT_FILE)
}

/// Writes the request in one move (temporary name, then rename): the bridge's watcher never sees a
/// partial file.
fn write_request(workspace: &Path, request_id: &str) -> std::io::Result<()> {
    let path = request_file(workspace, request_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(TEMP_EXTENSION);
    std::fs::write(&temporary, EMPTY_REQUEST_BODY)?;
    std::fs::rename(&temporary, &path)
}

fn read_json(path: &Path) -> Option<Value> {
    let text = aidp_core::text::read_text_lenient(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn output(report: &UnityRefreshReport) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput::text(serde_json::to_string(report)?))
}

#[cfg(test)]
#[path = "../../../tests/unit/flow_unity_refresh_tests.rs"]
mod tests;
