//! End-to-end tests for LOC-33's ambient-state CLI surface: warnings returned
//! instead of printed (scenarios 8-9), `scope resolve`'s pure/write split
//! observed from the CLI (scenario 10), and `run record-resume` still writing
//! the run's vault index note through the injected-vault call path
//! (scenario 11).

use crate::common;
use aidp_testkit::TempWorkspace;
use std::fs;

/// Path to a fixture shipped under `tests/fixtures/ambient_state/`.
fn fixture(name: &str) -> std::path::PathBuf {
    common::crate_dir()
        .join("tests/fixtures/ambient_state")
        .join(name)
}

/// Scenario 8: a `when:` `eval_condition` cannot parse surfaces in the `run
/// start` envelope's `warnings` array, naming the offending stage, and never
/// reaches stderr as the word `unparseable`.
#[test]
fn unparseable_when_warns_in_envelope_not_stderr() {
    let ws = TempWorkspace::new("ambient-state-when");

    let mut cmd = common::aidp(&ws);
    cmd.arg("run")
        .arg("start")
        .arg("--pipeline")
        .arg(fixture("unparseable_when.yaml"))
        .arg("--workspace-root")
        .arg(".")
        .arg("--no-vault");

    let output = cmd.output().expect("run start should execute");
    assert!(
        output.status.success(),
        "run start should exit 0 even with an unparseable when:, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let envelope: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("run start should print valid JSON");

    let warnings = envelope["warnings"]
        .as_array()
        .expect("warnings should be an array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().is_some_and(|s| s.contains("broken_when"))),
        "warnings should name the faulty stage `broken_when`, got: {warnings:?}"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unparseable"),
        "stderr should not contain 'unparseable' any more, got: {stderr}"
    );
}

/// Scenario 9: an undeclared `--input` surfaces in `warnings` (naming the key),
/// stderr no longer carries the old "is not declared by this pipeline" text,
/// and the rest of the envelope's contract keys are all still present.
#[test]
fn undeclared_input_warns_in_envelope_and_keeps_envelope_shape() {
    let ws = TempWorkspace::new("ambient-state-input");

    let mut cmd = common::aidp(&ws);
    cmd.arg("run")
        .arg("start")
        .arg("--pipeline")
        .arg(fixture("scoped_stage.yaml"))
        .arg("--input")
        .arg("bidule=42")
        .arg("--workspace-root")
        .arg(".")
        .arg("--no-vault");

    let output = cmd.output().expect("run start should execute");
    assert!(
        output.status.success(),
        "run start should exit 0 with an undeclared input, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let envelope: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("run start should print valid JSON");

    let warnings = envelope["warnings"]
        .as_array()
        .expect("warnings should be an array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().is_some_and(|s| s.contains("bidule"))),
        "warnings should mention the undeclared input `bidule`, got: {warnings:?}"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("is not declared by this pipeline"),
        "stderr should not contain the old undeclared-input message, got: {stderr}"
    );

    for key in [
        "run_id",
        "run_dir",
        "script_path",
        "awaiting_plan",
        "gates",
        "auto_approved_gates",
        "last_workflow_status",
    ] {
        assert!(
            envelope.get(key).is_some(),
            "envelope should still carry key `{key}`, got: {envelope}"
        );
    }
}

/// Scenario 10: `scope resolve` still writes `scope.json` to the stage
/// directory with the pure/write split behind it, and the written file
/// carries the contract keys.
#[test]
fn scope_resolve_writes_scope_json_with_contract_keys() {
    let ws = TempWorkspace::new("ambient-state-scope");

    let mut init_cmd = common::aidp(&ws);
    init_cmd
        .arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg(fixture("scoped_stage.yaml"))
        .arg("--workspace-root")
        .arg(".")
        .arg("--no-vault");
    let init_output = init_cmd.output().expect("run init should execute");
    assert!(
        init_output.status.success(),
        "run init should succeed, stderr: {}",
        String::from_utf8_lossy(&init_output.stderr)
    );
    let run_id = String::from_utf8_lossy(&init_output.stdout)
        .trim()
        .to_string();
    assert!(!run_id.is_empty(), "run init should print a run_id");

    let mut resolve_cmd = common::aidp(&ws);
    resolve_cmd
        .arg("scope")
        .arg("resolve")
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("implement")
        .arg("--workspace-root")
        .arg(".");

    let resolve_output = resolve_cmd.output().expect("scope resolve should execute");
    assert!(
        resolve_output.status.success(),
        "scope resolve should succeed, stderr: {}",
        String::from_utf8_lossy(&resolve_output.stderr)
    );

    let resolved: serde_json::Value = serde_json::from_slice(&resolve_output.stdout)
        .expect("scope resolve stdout should be JSON");
    for key in ["stage", "mode", "stage_dir", "allowed", "denied"] {
        assert!(
            resolved.get(key).is_some(),
            "scope.json should carry key `{key}`, got: {resolved}"
        );
    }
    assert_eq!(
        resolved["stage"].as_str(),
        Some("implement"),
        "scope.json's stage should be implement"
    );
    assert_eq!(
        resolved["mode"].as_str(),
        Some("tests"),
        "scope.json's mode should reflect the fixture's declared scope"
    );

    let stage_dir = resolved["stage_dir"]
        .as_str()
        .expect("stage_dir should be a string");
    let scope_json_path = ws.path().join(stage_dir).join("scope.json");
    assert!(
        scope_json_path.exists(),
        "scope.json should be written to the stage_dir, expected at {scope_json_path:?}"
    );

    let written: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&scope_json_path).expect("scope.json should be readable"),
    )
    .expect("scope.json should be valid JSON");
    assert_eq!(
        resolved, written,
        "stdout and the written scope.json should be identical"
    );
}

/// Scenario 11: with a real (test) vault, `run record-resume` still triggers
/// the run's `_<run_id>.md` index note, proving the note-writing side effect
/// survived the `save`/`save_state` split behind `load_in`/`record_resume`.
#[test]
fn record_resume_with_vault_still_writes_run_index_note() {
    let ws = TempWorkspace::new("ambient-state-vault-note");

    let mut init_cmd = common::aidp(&ws);
    init_cmd
        .arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg(common::crate_dir().join("tests/fixtures/t05-minimal.yaml"))
        .arg("--workspace-root")
        .arg(".");
    let init_output = init_cmd.output().expect("run init should execute");
    assert!(
        init_output.status.success(),
        "run init should succeed, stderr: {}",
        String::from_utf8_lossy(&init_output.stderr)
    );
    let run_id = String::from_utf8_lossy(&init_output.stdout)
        .trim()
        .to_string();
    assert!(!run_id.is_empty(), "run init should print a run_id");

    let stderr = String::from_utf8_lossy(&init_output.stderr);
    let run_dir_line = stderr
        .lines()
        .find(|l| l.starts_with("run_dir:"))
        .expect("run init stderr should print run_dir:");
    let run_dir = run_dir_line
        .trim_start_matches("run_dir:")
        .trim()
        .to_string();
    let run_dir_path = std::path::PathBuf::from(&run_dir);
    let note_path = run_dir_path.join(format!("_{run_id}.md"));
    assert!(
        note_path.exists(),
        "run init itself should already have written the run's index note at {note_path:?}"
    );
    let note_before =
        fs::read_to_string(&note_path).expect("run index note should be readable after init");

    let mut resume_cmd = common::aidp(&ws);
    resume_cmd
        .arg("run")
        .arg("record-resume")
        .arg("--run")
        .arg(&run_id)
        .arg("--workflow-run-id")
        .arg("wf-loc33")
        .arg("--workspace-root")
        .arg(".");
    let resume_output = resume_cmd.output().expect("record-resume should execute");
    assert!(
        resume_output.status.success(),
        "record-resume should succeed, stderr: {}",
        String::from_utf8_lossy(&resume_output.stderr)
    );

    assert!(
        note_path.exists(),
        "the run index note should still exist after record-resume at {note_path:?}"
    );
    let note_after =
        fs::read_to_string(&note_path).expect("run index note should be readable after resume");
    assert_ne!(
        note_before, note_after,
        "record-resume should have refreshed the note (it now carries workflow_run_id)"
    );
    assert!(
        note_after.contains("wf-loc33"),
        "the refreshed note should mention the recorded workflow_run_id, got: {note_after}"
    );
}
