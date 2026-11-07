//! End-to-end tests for rebuilding a run's resume anchor from a Workflow result
//! file with `aidp run record-resume --from-result`, when the anchor was never
//! recorded while the run was live.

use crate::common;
use aidp_testkit::TempWorkspace;
use std::fs;
use std::path::PathBuf;
use std::process::Output;

const GATE: &str = "synthesize";
const RECOVERED_RUN_ID: &str = "wf-cold-1";

struct StartedRun {
    run_id: String,
    run_dir: PathBuf,
}

impl StartedRun {
    fn run_state_path(&self) -> PathBuf {
        self.run_dir.join("run-state.json")
    }

    fn result_path(&self) -> PathBuf {
        self.run_dir.join("workflow-result.json")
    }

    fn read_run_state(&self) -> serde_json::Value {
        let raw = fs::read_to_string(self.run_state_path()).expect("run-state.json is readable");
        serde_json::from_str(&raw).expect("run-state.json is valid JSON")
    }
}

fn start_study_run(ws: &TempWorkspace) -> StartedRun {
    let mut cmd = common::aidp_with_fixture_skill(ws);
    cmd.args(["run", "start", "--pipeline", "study", "--input", "topic=x"])
        .args(["--workspace-root", ".", "--no-vault"]);
    let output = cmd.output().expect("run start should execute");
    assert!(output.status.success(), "run start should succeed");

    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("run start prints JSON");
    StartedRun {
        run_id: envelope["run_id"].as_str().expect("run_id").to_string(),
        run_dir: ws
            .path()
            .join(envelope["run_dir"].as_str().expect("run_dir")),
    }
}

fn write_awaiting_gate_result(run: &StartedRun, gate: &str) {
    let result = serde_json::json!({
        "result": {
            "status": "awaiting_gate",
            "gate": gate,
            "runId": RECOVERED_RUN_ID,
            "results": {}
        }
    });
    fs::write(run.result_path(), result.to_string()).expect("write workflow-result.json");
}

fn record_resume(ws: &TempWorkspace, run: &StartedRun, extra: &[&str]) -> Output {
    let mut cmd = common::aidp_with_fixture_skill(ws);
    cmd.args(["run", "record-resume", "--run", &run.run_id])
        .args(["--workspace-root", "."])
        .args(extra);
    cmd.output().expect("record-resume should execute")
}

fn from_result_args(run: &StartedRun) -> Vec<String> {
    vec![
        "--from-result".to_string(),
        run.result_path().display().to_string(),
    ]
}

fn record_from_result(ws: &TempWorkspace, run: &StartedRun, extra: &[&str]) -> Output {
    let from_result = from_result_args(run);
    let mut args: Vec<&str> = from_result.iter().map(String::as_str).collect();
    args.extend_from_slice(extra);
    record_resume(ws, run, &args)
}

fn stdout_json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout is JSON")
}

/// A run with no recorded anchor recovers the runId and the pending gate from an
/// enveloped `awaiting_gate` result file.
#[test]
fn cold_resume_rebuilds_anchor_from_enveloped_result() {
    let ws = common::workspace();
    let run = start_study_run(&ws);
    let before = run.read_run_state();
    assert!(before["workflow_run_id"].is_null());
    assert!(before["pending_gate"].is_null());

    write_awaiting_gate_result(&run, GATE);
    let output = record_from_result(&ws, &run, &[]);
    assert!(
        output.status.success(),
        "record-resume --from-result succeeds"
    );

    let printed = stdout_json(&output);
    assert_eq!(printed["workflow_run_id"], RECOVERED_RUN_ID);
    assert_eq!(printed["pending_gate"], GATE);

    let after = run.read_run_state();
    assert_eq!(after["workflow_run_id"], RECOVERED_RUN_ID);
    assert_eq!(after["pending_gate"], GATE);
}

/// After the anchor is recovered, approving the gate hands the driver the
/// recovered runId to resume from and clears the pending gate.
#[test]
fn cold_resume_then_approve_resumes_from_recovered_run_id() {
    let ws = common::workspace();
    let run = start_study_run(&ws);
    write_awaiting_gate_result(&run, GATE);
    assert!(record_from_result(&ws, &run, &[]).status.success());

    let mut cmd = common::aidp_with_fixture_skill(&ws);
    cmd.args(["gate", "approve", "--run", &run.run_id, "--gate", GATE])
        .args(["--workspace-root", "."]);
    let output = cmd.output().expect("gate approve should execute");
    assert!(output.status.success(), "gate approve succeeds");

    let resume = stdout_json(&output);
    assert_eq!(resume["resumeFromRunId"], RECOVERED_RUN_ID);
    let approved: Vec<&str> = resume["args"]["approved_gates"]
        .as_array()
        .expect("approved_gates is an array")
        .iter()
        .filter_map(|gate| gate.as_str())
        .collect();
    assert!(approved.contains(&GATE), "approved gates: {approved:?}");

    let after = run.read_run_state();
    assert!(after["pending_gate"].is_null(), "pending gate is cleared");
    assert_eq!(after["workflow_run_id"], RECOVERED_RUN_ID);
}

/// An explicit `--workflow-run-id` wins over the runId held in the result file.
#[test]
fn cold_resume_explicit_flags_win_over_file() {
    let ws = common::workspace();
    let run = start_study_run(&ws);
    write_awaiting_gate_result(&run, GATE);

    let output = record_from_result(&ws, &run, &["--workflow-run-id", "wf-explicit"]);
    assert!(output.status.success(), "record-resume succeeds");

    let after = run.read_run_state();
    assert_eq!(after["workflow_run_id"], "wf-explicit");
    assert_eq!(after["pending_gate"], GATE);
}

/// A gate approved beforehand is not reopened when the result file names it again.
#[test]
fn cold_resume_does_not_reopen_approved_gate() {
    let ws = common::workspace();
    let run = start_study_run(&ws);
    let approve = record_resume(&ws, &run, &["--approved-gate", GATE]);
    assert!(approve.status.success(), "approving the gate succeeds");

    write_awaiting_gate_result(&run, GATE);
    let output = record_from_result(&ws, &run, &[]);
    assert!(
        output.status.success(),
        "record-resume --from-result succeeds"
    );

    let after = run.read_run_state();
    assert!(
        after["pending_gate"].is_null(),
        "approved gate stays closed"
    );
    assert_eq!(after["workflow_run_id"], RECOVERED_RUN_ID);
}

/// A missing file and a malformed file both fail, name the path on stderr and
/// leave run-state.json byte for byte unchanged.
#[test]
fn cold_resume_missing_or_malformed_result_fails() {
    let ws = common::workspace();
    let run = start_study_run(&ws);
    let state_before = fs::read(run.run_state_path()).expect("read run-state.json");

    let missing = run.result_path();
    let output = record_from_result(&ws, &run, &[]);
    assert!(!output.status.success(), "a missing file fails");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("workflow-result.json"),
        "stderr names the path {}: {stderr}",
        missing.display()
    );

    fs::write(run.result_path(), "not json").expect("write malformed result");
    let output = record_from_result(&ws, &run, &[]);
    assert!(!output.status.success(), "a malformed file fails");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("workflow-result.json"),
        "stderr names the path: {stderr}"
    );

    let state_after = fs::read(run.run_state_path()).expect("read run-state.json");
    assert_eq!(state_before, state_after, "run-state.json is unchanged");
}
