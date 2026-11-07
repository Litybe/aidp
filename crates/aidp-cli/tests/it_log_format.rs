use crate::common;
use common::*;
use std::process::Output;

const RUST_LOG_VAR: &str = "RUST_LOG";
const INFO_LEVEL: &str = "info";
const TRACE_LEVEL: &str = "trace";
const WARNING_PREFIX: &str = "warning:";
const PHASE_KEY: &str = "\"phase\"";
const ANSI_ESCAPE: &str = "\x1b[";
const LEVEL_KEY: &str = "level";
const TIMESTAMP_KEY: &str = "timestamp";
const TARGET_KEY: &str = "target";
const DEBUG_MARKER: &str = "DEBUG";
const TRACE_MARKER: &str = "TRACE";

fn validate_with_log(level: &str) -> Output {
    let ws = workspace();
    aidp_with_fixture_skill(&ws)
        .env(RUST_LOG_VAR, level)
        .args(["validate", "dev"])
        .output()
        .expect("aidp runs")
}

fn is_log_line(line: &str) -> bool {
    !line.trim().is_empty() && !line.starts_with(WARNING_PREFIX) && !line.contains(PHASE_KEY)
}

#[test]
fn piped_stderr_is_json_without_ansi() {
    let output = validate_with_log(INFO_LEVEL);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains(ANSI_ESCAPE), "ANSI found: {stderr}");
    let log_lines: Vec<&str> = stderr.lines().filter(|l| is_log_line(l)).collect();
    assert!(!log_lines.is_empty(), "no log line in: {stderr}");
    for line in log_lines {
        let value: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
        for key in [TIMESTAMP_KEY, TARGET_KEY] {
            assert!(value.get(key).is_some(), "missing {key}: {line}");
        }
        assert!(value.get(LEVEL_KEY).is_some(), "missing level: {line}");
    }
}

#[test]
fn trace_level_logs_without_touching_stdout() {
    let output = validate_with_log(TRACE_LEVEL);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(TRACE_MARKER) || stderr.contains(DEBUG_MARKER),
        "no trace or debug line in: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("stdout stays JSON");
}

#[test]
fn default_stderr_carries_no_log_line() {
    let ws = workspace();
    let output = aidp_with_fixture_skill(&ws)
        .env_remove(RUST_LOG_VAR)
        .env_remove(AIDP_LOG_VAR)
        .args(["validate", "dev"])
        .output()
        .expect("aidp runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains(COMMAND_STARTED),
        "unexpected log: {stderr}"
    );
}
