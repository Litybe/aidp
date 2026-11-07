//! End-to-end tests for malformed or hostile external input reaching the `aidp` binary:
//! a cyclic pipeline, a corrupt pricing grid, a blocked ticket board and a blocked vault note.
//! Each scenario must fail loudly or degrade with a warning, never panic or pass silently.

use crate::common;
use aidp_testkit::TempWorkspace;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const CYCLIC_PIPELINE_FILE: &str = "cyc.yaml";
const CYCLIC_PIPELINE_YAML: &str = include_str!("fixtures/external-input/cyclic-pipeline.yaml");
const SINGLE_STAGE_PIPELINE_FILE: &str = "ok.yaml";
const SINGLE_STAGE_PIPELINE_YAML: &str =
    include_str!("fixtures/external-input/single-stage-pipeline.yaml");
const INVALID_PRICES_FILE: &str = "invalid-prices.json";
const INVALID_PRICES_JSON: &str = include_str!("fixtures/external-input/invalid-prices.json");
const RECONCILE_RESULT_FILE: &str = "result.json";
const RECONCILE_RESULT_JSON: &str = include_str!("fixtures/external-input/reconcile-result.json");

const VALIDATE_CMD: &str = "validate";
const ALL_FLAG: &str = "--all";
const RUN_CMD: &str = "run";
const START_OP: &str = "start";
const RECONCILE_OP: &str = "reconcile";
const TICKET_CMD: &str = "ticket";
const VAULT_CMD: &str = "vault";
const NEW_OP: &str = "new";
const REINDEX_OP: &str = "reindex";
const PIPELINE_FLAG: &str = "--pipeline";
const WORKSPACE_ROOT_FLAG: &str = "--workspace-root";
const NO_VAULT_FLAG: &str = "--no-vault";
const RUN_FLAG: &str = "--run";
const FROM_FLAG: &str = "--from";
const VAULT_FLAG: &str = "--vault";
const SERVICE_FLAG: &str = "--service";
const SUMMARY_FLAG: &str = "--summary";
const ALLOW_UNKNOWN_SERVICE_FLAG: &str = "--allow-unknown-service";
const CURRENT_DIR: &str = ".";

const MODEL_PRICES_ENV: &str = "AIDP_MODEL_PRICES";
const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";

const CYCLE_ERROR_FRAGMENT: &str = "cycle or unresolved dep";
const CYCLIC_STAGE_A: &str = "\"a\"";
const CYCLIC_STAGE_B: &str = "\"b\"";
const SINGLE_STAGE_NAME: &str = "a";
const SUMMARY_HEADING_PREFIX: &str = "# Summary ";
const SUCCESS_STATUS: &str = "success";
const UNAVAILABLE_SOURCE: &str = "unavailable";

const WORKFLOW_SCRIPT_FILE: &str = "workflow.js";
const RUN_IN_FLIGHT_FILE: &str = "run-in-flight";
const RUN_STATE_FILE: &str = "run-state.json";

const DEMO_SERVICE: &str = "demo";
const FIRST_SUMMARY: &str = "first";
const SECOND_SUMMARY: &str = "second";
const TODO_DIR: &str = "TODO";
const BOARD_FILE: &str = "_TODO.md";
const BOARD_RELATIVE_PATH: &str = "TODO/_TODO.md";

const RUN_NOTE_PREFIX: &str = "_";
const RUN_NOTE_SUFFIX: &str = ".md";

const RUN_ID_FIELD: &str = "run_id";
const RUN_DIR_FIELD: &str = "run_dir";
const KEY_FIELD: &str = "key";
const PATH_FIELD: &str = "path";
const WARNINGS_FIELD: &str = "warnings";
const ARCHIVED_FIELD: &str = "archived";
const OK_FIELD: &str = "ok";
const FAILED_FIELD: &str = "failed";
const STAGES_FIELD: &str = "stages";
const NAME_FIELD: &str = "name";
const STATUS_FIELD: &str = "status";
const USAGE_FIELD: &str = "usage";
const SOURCE_FIELD: &str = "source";
const COST_FIELD: &str = "cost";

/// Replaces the file at `path` with a directory of the same name, so a later
/// write through that path fails whatever the process's privileges — a
/// read-only bit does not stop a root or `CAP_DAC_OVERRIDE` process from
/// writing, but a directory sitting where a file is expected blocks every
/// writer identically.
fn block_with_directory(path: &Path) {
    fs::remove_file(path).expect("removing the file to block should succeed");
    fs::create_dir_all(path).expect("blocking the path with a directory should succeed");
}

/// Writes `content` to `name` at the workspace root and returns its absolute path.
fn write_fixture(ws: &TempWorkspace, name: &str, content: &str) -> PathBuf {
    let path = ws.child(name);
    fs::write(&path, content).expect("writing the fixture should succeed");
    path
}

/// Every file named `name` anywhere below `root`.
fn files_named(root: &Path, name: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_named(&path, name));
        } else if path.file_name().and_then(|file| file.to_str()) == Some(name) {
            found.push(path);
        }
    }
    found
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout should be JSON ({error}), got: {}",
            stdout_text(output)
        )
    })
}

/// The non-empty `warnings` array of a ticket command's JSON envelope.
fn non_empty_warnings(envelope: &Value) -> Vec<String> {
    let warnings: Vec<String> = envelope[WARNINGS_FIELD]
        .as_array()
        .unwrap_or_else(|| panic!("`warnings` should be an array, got: {envelope}"))
        .iter()
        .map(|warning| warning.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        !warnings.is_empty(),
        "warnings should not be empty: {envelope}"
    );
    warnings
}

/// Runs `aidp run start --no-vault` on the pipeline at `pipeline_path`.
fn run_start(ws: &TempWorkspace, pipeline_path: &Path) -> Output {
    aidp(ws)
        .args([RUN_CMD, START_OP, PIPELINE_FLAG])
        .arg(pipeline_path)
        .args([WORKSPACE_ROOT_FLAG, CURRENT_DIR, NO_VAULT_FLAG])
        .output()
        .expect("run start should execute")
}

/// Runs `aidp ticket new --vault . --service demo` with the given summary.
fn ticket_new(ws: &TempWorkspace, summary: &str) -> Output {
    aidp(ws)
        .args([TICKET_CMD, NEW_OP, VAULT_FLAG, CURRENT_DIR])
        .args([SERVICE_FLAG, DEMO_SERVICE, SUMMARY_FLAG, summary])
        .arg(ALLOW_UNKNOWN_SERVICE_FLAG)
        .output()
        .expect("ticket new should execute")
}

/// A vault holding one ticket, whose generated board is then returned by path.
fn vault_with_one_ticket(ws: &TempWorkspace) -> PathBuf {
    let output = ticket_new(ws, FIRST_SUMMARY);
    assert!(
        output.status.success(),
        "seeding ticket new should succeed, stderr: {}",
        stderr_text(&output)
    );
    let board = ws.child(TODO_DIR).join(BOARD_FILE);
    assert!(board.is_file(), "ticket new should generate {BOARD_FILE}");
    board
}

#[test]
fn validate_rejects_a_cyclic_pipeline_naming_both_stages() {
    let ws = workspace();
    write_fixture(&ws, CYCLIC_PIPELINE_FILE, CYCLIC_PIPELINE_YAML);

    let output = aidp(&ws)
        .args([VALIDATE_CMD, &format!("./{CYCLIC_PIPELINE_FILE}")])
        .output()
        .expect("validate should execute");

    assert_eq!(
        output.status.code(),
        Some(1),
        "a cyclic pipeline is invalid"
    );
    let stderr = stderr_text(&output);
    for fragment in [CYCLE_ERROR_FRAGMENT, CYCLIC_STAGE_A, CYCLIC_STAGE_B] {
        assert!(
            stderr.contains(fragment),
            "stderr should contain {fragment}, got: {stderr}"
        );
    }
}

#[test]
fn run_start_refuses_a_cyclic_pipeline() {
    let ws = workspace();
    let pipeline_path = write_fixture(&ws, CYCLIC_PIPELINE_FILE, CYCLIC_PIPELINE_YAML);

    let output = run_start(&ws, &pipeline_path);

    assert_ne!(
        output.status.code(),
        Some(0),
        "run start must refuse a cycle"
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains(CYCLE_ERROR_FRAGMENT),
        "stderr should name the cycle, got: {stderr}"
    );
    let scripts = files_named(ws.path(), WORKFLOW_SCRIPT_FILE);
    assert!(
        scripts.is_empty(),
        "no workflow script may be emitted, found: {scripts:?}"
    );
}

#[test]
fn reconcile_degrades_on_an_invalid_price_grid() {
    let ws = workspace();
    let pipeline_path = write_fixture(&ws, SINGLE_STAGE_PIPELINE_FILE, SINGLE_STAGE_PIPELINE_YAML);
    let prices_path = write_fixture(&ws, INVALID_PRICES_FILE, INVALID_PRICES_JSON);
    let result_path = write_fixture(&ws, RECONCILE_RESULT_FILE, RECONCILE_RESULT_JSON);

    let started = run_start(&ws, &pipeline_path);
    assert!(
        started.status.success(),
        "run start should succeed, stderr: {}",
        stderr_text(&started)
    );
    let envelope = stdout_json(&started);
    let run_id = envelope[RUN_ID_FIELD]
        .as_str()
        .expect("run start envelope should carry run_id")
        .to_string();
    let run_dir = ws.path().join(
        envelope[RUN_DIR_FIELD]
            .as_str()
            .expect("run start envelope should carry run_dir"),
    );
    assert!(
        !files_named(ws.path(), RUN_IN_FLIGHT_FILE).is_empty(),
        "run start should leave a run-in-flight marker"
    );

    let output = aidp(&ws)
        .env(MODEL_PRICES_ENV, &prices_path)
        .args([RUN_CMD, RECONCILE_OP, RUN_FLAG, &run_id, FROM_FLAG])
        .arg(&result_path)
        .args([WORKSPACE_ROOT_FLAG, CURRENT_DIR])
        .output()
        .expect("run reconcile should execute");

    let stderr = stderr_text(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "reconcile must degrade, not fail, stderr: {stderr}"
    );
    let stdout = stdout_text(&output);
    assert!(
        stdout.starts_with(&format!("{SUMMARY_HEADING_PREFIX}{run_id}")),
        "the summary should still print, got: {stdout}"
    );
    let prices_display = prices_path.display().to_string();
    let decoded_stderr = stderr.replace("\\\\", "\\");
    for fragment in [MODEL_PRICES_ENV, prices_display.as_str()] {
        assert!(
            decoded_stderr.contains(fragment),
            "stderr should name {fragment}, got: {stderr}"
        );
    }

    let state: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join(RUN_STATE_FILE)).expect("run-state.json should exist"),
    )
    .expect("run-state.json should be JSON");
    let stages = state[STAGES_FIELD]
        .as_array()
        .expect("run-state should list its stages");
    let reconciled = stages
        .iter()
        .find(|stage| stage[NAME_FIELD] == SINGLE_STAGE_NAME)
        .expect("stage a should be in run-state");
    assert_eq!(reconciled[STATUS_FIELD], SUCCESS_STATUS);
    for stage in stages {
        assert_eq!(
            stage[USAGE_FIELD][SOURCE_FIELD], UNAVAILABLE_SOURCE,
            "every stage's usage should be unavailable: {stage}"
        );
    }
    assert!(
        state.get(COST_FIELD).is_none_or(Value::is_null),
        "no cost may be computed from an invalid grid: {}",
        state[COST_FIELD]
    );
    let markers = files_named(ws.path(), RUN_IN_FLIGHT_FILE);
    assert!(
        markers.is_empty(),
        "reconcile should clear the run-in-flight marker, found: {markers:?}"
    );
}

#[test]
fn ticket_new_warns_when_the_board_path_is_blocked() {
    let ws = workspace();
    let board = vault_with_one_ticket(&ws);
    block_with_directory(&board);

    let output = ticket_new(&ws, SECOND_SUMMARY);

    let stderr = stderr_text(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a board write failure is a warning, stderr: {stderr}"
    );
    let envelope = stdout_json(&output);
    assert!(envelope.get(KEY_FIELD).is_some(), "missing key: {envelope}");
    assert!(
        envelope.get(PATH_FIELD).is_some(),
        "missing path: {envelope}"
    );
    let warnings = non_empty_warnings(&envelope);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains(BOARD_RELATIVE_PATH)),
        "a warning should name {BOARD_RELATIVE_PATH}: {warnings:?}"
    );
    assert!(
        stderr.contains(BOARD_FILE),
        "stderr should name {BOARD_FILE}, got: {stderr}"
    );
}

#[test]
fn ticket_reindex_warns_when_the_board_path_is_blocked() {
    let ws = workspace();
    let board = vault_with_one_ticket(&ws);
    block_with_directory(&board);

    let output = aidp(&ws)
        .args([TICKET_CMD, REINDEX_OP, VAULT_FLAG, CURRENT_DIR])
        .output()
        .expect("ticket reindex should execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "a board write failure is a warning, stderr: {}",
        stderr_text(&output)
    );
    let envelope = stdout_json(&output);
    assert!(
        envelope.get(ARCHIVED_FIELD).is_some(),
        "missing archived: {envelope}"
    );
    non_empty_warnings(&envelope);
}

#[test]
fn vault_reindex_reports_a_run_note_that_could_not_be_written() {
    let ws = workspace();
    let pipeline_path = write_fixture(&ws, SINGLE_STAGE_PIPELINE_FILE, SINGLE_STAGE_PIPELINE_YAML);

    let started = aidp(&ws)
        .args([RUN_CMD, START_OP, PIPELINE_FLAG])
        .arg(&pipeline_path)
        .args([WORKSPACE_ROOT_FLAG, CURRENT_DIR, VAULT_FLAG, CURRENT_DIR])
        .output()
        .expect("run start should execute");
    assert!(
        started.status.success(),
        "run start should succeed, stderr: {}",
        stderr_text(&started)
    );
    let envelope = stdout_json(&started);
    let run_id = envelope[RUN_ID_FIELD]
        .as_str()
        .expect("run start envelope should carry run_id")
        .to_string();
    let run_dir = ws.path().join(
        envelope[RUN_DIR_FIELD]
            .as_str()
            .expect("run start envelope should carry run_dir"),
    );
    let note = run_dir.join(format!("{RUN_NOTE_PREFIX}{run_id}{RUN_NOTE_SUFFIX}"));
    assert!(note.is_file(), "run start should generate the run note");
    block_with_directory(&note);

    let output = aidp(&ws)
        .args([VAULT_CMD, REINDEX_OP, VAULT_FLAG, CURRENT_DIR])
        .output()
        .expect("vault reindex should execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "reindex is best-effort, stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains(&run_dir.display().to_string()),
        "stderr should name the run dir whose notes could not be written, got: {stderr}"
    );
    let note_name = note
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the note path should have a file name");
    assert!(
        stderr.contains(note_name),
        "stderr should name the blocked note {note_name}, got: {stderr}"
    );
}

#[test]
fn validate_all_keeps_every_template_green() {
    let ws = workspace();
    let root = skill_root();

    let output = aidp(&ws)
        .env(SKILL_ROOT_ENV, &root)
        .args([VALIDATE_CMD, ALL_FLAG, WORKSPACE_ROOT_FLAG])
        .arg(&root)
        .output()
        .expect("validate --all should execute");

    assert_eq!(
        output.status.code(),
        Some(0),
        "every template should validate, stderr: {}",
        stderr_text(&output)
    );
    let report = stdout_json(&output);
    assert_eq!(report[OK_FIELD], true, "report: {report}");
    assert_eq!(report[FAILED_FIELD], 0, "report: {report}");
}
