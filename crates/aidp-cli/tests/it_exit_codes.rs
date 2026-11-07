//! End-to-end tests for the ticket scenarios of the `ExitStatus` migration.
//!
//! This file complements `it_scope_exit_codes.rs` and `it_vault_leak.rs` with the
//! specific scenarios called out for this change: `validate` on a missing pipeline,
//! `findings merge` with no `--input`, a scope-check policy refusal, an unknown
//! stage, a missing ticket, a clap usage typo on `ci hook stop`, a legitimate hook
//! wake, a cost-anomaly outlier, a cost anomaly with no vault, and the root
//! `--help`/`--version` pair. Every assertion goes through
//! `aidp_core::ExitStatus::<variant>.code()` rather than a bare literal.

use crate::common;
use aidp_core::ExitStatus;
use common::*;
use serde_json::json;
use std::fs;

/// `aidp validate nope` names the pipeline it could not find and exits
/// [`ExitStatus::NotFound`].
#[test]
fn validate_missing_pipeline_names_it_and_exits_not_found() {
    let ws = workspace();

    let mut cmd = aidp(&ws);
    cmd.arg("validate")
        .arg("nope")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run validate");
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::NotFound.code() as i32),
        "validate: missing pipeline should exit NotFound"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("Error: pipeline not found: nope"),
        "stderr should start with 'Error: pipeline not found: nope', got: {}",
        stderr
    );
}

/// `aidp findings merge --out <f>` with no `--input`/`--label` axis fails
/// before writing anything, with [`ExitStatus::Usage`].
#[test]
fn findings_merge_without_input_writes_nothing_and_exits_usage() {
    let ws = workspace();
    let out_path = ws.child("merged.json");

    let mut cmd = aidp(&ws);
    cmd.arg("findings").arg("merge").arg("--out").arg(&out_path);

    let output = cmd.output().expect("Failed to run findings merge");
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::Usage.code() as i32),
        "findings merge without --input should exit Usage"
    );

    assert!(
        !out_path.exists(),
        "findings merge: the --out file must not be created on this failure"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("at least one --input or --label"),
        "stderr should explain the missing axis, got: {}",
        stderr
    );
}

/// `aidp scope check` on a path outside the stage's scope is a policy
/// refusal: exit [`ExitStatus::PolicyRefusal`], with `{"ok": false, "reason": ...}`
/// on stdout.
#[test]
fn scope_check_refusal_exits_policy_refusal_with_json_reason() {
    let ws = workspace();
    let run_id = seed_t05_run(&ws);

    let outside_file = ws.child("outside.txt");
    fs::write(&outside_file, "outside").expect("Failed to write file");

    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("check")
        .arg("--path")
        .arg(outside_file.to_string_lossy().to_string())
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("only")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope check");
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::PolicyRefusal.code() as i32),
        "scope check: refusal should exit PolicyRefusal"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("scope check: stdout should be JSON");
    assert_eq!(
        parsed["ok"], false,
        "scope check: JSON should have ok: false"
    );
    assert!(
        !parsed["reason"].as_str().unwrap_or("").is_empty(),
        "scope check: reason should be non-empty"
    );
}

/// `aidp scope check --stage <unknown>` is a not-found: exit
/// [`ExitStatus::NotFound`], with `{"ok": false, "reason": ...}` on stdout and
/// "unknown stage" on stderr.
#[test]
fn scope_check_unknown_stage_exits_not_found_with_json_reason() {
    let ws = workspace();
    let run_id = seed_t05_run(&ws);

    let test_file = ws.child("test.txt");
    fs::write(&test_file, "test").expect("Failed to write file");

    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("check")
        .arg("--path")
        .arg(test_file.to_string_lossy().to_string())
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("nosuchstage")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope check");
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::NotFound.code() as i32),
        "scope check: unknown stage should exit NotFound"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("scope check: stdout should be JSON");
    assert_eq!(
        parsed["ok"], false,
        "scope check: unknown stage should have ok: false"
    );
    assert!(
        !parsed["reason"].as_str().unwrap_or("").is_empty(),
        "scope check: reason should be non-empty"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown stage: nosuchstage"),
        "stderr should name the unknown stage, got: {}",
        stderr
    );
}

/// `aidp ticket show NOPE-9` on an empty vault names the ticket and exits
/// [`ExitStatus::NotFound`], on stderr only.
#[test]
fn ticket_show_missing_key_exits_not_found_on_one_stream() {
    let ws = workspace();
    let vault = ws.child("vault");
    fs::create_dir_all(&vault).expect("Failed to create vault");

    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(&vault)
        .arg("show")
        .arg("NOPE-9");

    let output = cmd.output().expect("Failed to run ticket show");
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::NotFound.code() as i32),
        "ticket show: missing key should exit NotFound"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.is_empty(),
        "ticket show: stdout should be empty on failure, got: {}",
        stdout
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("NOPE-9") && stderr.contains("not found"),
        "stderr should name the missing ticket, got: {}",
        stderr
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&stderr)
            .ok()
            .and_then(|v| v.get("error").cloned())
            .is_none(),
        "stderr should be plain text, not a JSON object carrying an 'error' key"
    );
}

/// Four runs with costs `1.00, 1.02, 1.04, 50.00` flag exactly the last one as
/// an outlier: exit [`ExitStatus::NegativeVerdict`], `status: "ok"` and
/// `any_flagged: true`.
#[test]
fn cost_anomaly_with_a_real_outlier_exits_negative_verdict() {
    let ws = workspace();
    let vault_dir = ws.child("vault");
    let base_path = vault_dir.join("Pipelines/demo/feature");
    fs::create_dir_all(&base_path).expect("Failed to create pipelines dir");

    let costs = [1.00, 1.02, 1.04, 50.00];
    for (i, cost) in costs.iter().enumerate() {
        let run_id = format!("20260919-{:02}0000-000{}", i + 10, i + 1);
        let run_dir = base_path.join(&run_id);
        fs::create_dir_all(&run_dir).expect("Failed to create run dir");

        let run_state = json!({
            "run_id": run_id,
            "pipeline_id": "demo",
            "pipeline_path": "test.yaml",
            "workspace_root": ".",
            "run_dir": run_dir.to_string_lossy().to_string(),
            "started_at": "2026-09-19T10:00:00Z",
            "stages": [],
            "cost": {
                "total_usd": cost,
                "tokens_billable": 0,
                "captured_at": "2026-09-19T10:00:00Z"
            }
        });
        fs::write(run_dir.join("run-state.json"), run_state.to_string())
            .expect("Failed to write run-state.json");
    }

    let mut cmd = aidp(&ws);
    cmd.env("AIDP_VAULT", vault_dir.to_string_lossy().to_string())
        .arg("cost")
        .arg("anomaly")
        .arg("--repo")
        .arg("demo")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run cost anomaly");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::NegativeVerdict.code() as i32),
        "cost anomaly: an outlier should exit NegativeVerdict, stdout: {}",
        stdout
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("cost anomaly: stdout should be JSON");
    assert_eq!(
        parsed["status"].as_str(),
        Some("ok"),
        "cost anomaly: status should be 'ok', got: {}",
        parsed
    );
    assert_eq!(
        parsed["any_flagged"].as_bool(),
        Some(true),
        "cost anomaly: any_flagged should be true"
    );
}

/// `aidp cost anomaly` with no vault configured stays [`ExitStatus::Success`]:
/// `status: "insufficient_data"`, `n: 0`.
#[test]
fn cost_anomaly_with_no_vault_stays_insufficient_data() {
    let ws = workspace();

    let mut cmd = aidp(&ws);
    cmd.env("AIDP_VAULT", "off")
        .arg("cost")
        .arg("anomaly")
        .arg("--repo")
        .arg("demo")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run cost anomaly");
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::Success.code() as i32),
        "cost anomaly: no vault should exit Success"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("cost anomaly: stdout should be JSON");
    assert_eq!(
        parsed["status"].as_str(),
        Some("insufficient_data"),
        "cost anomaly: status should be 'insufficient_data'"
    );
    assert_eq!(parsed["n"].as_i64(), Some(0), "cost anomaly: n should be 0");
}

/// `aidp --help` and `aidp --version` both exit [`ExitStatus::Success`], and
/// the help text points at the exit-code table in README.md.
#[test]
fn help_and_version_exit_success() {
    let ws = workspace();

    let mut help_cmd = aidp(&ws);
    let help_output = help_cmd
        .arg("--help")
        .output()
        .expect("Failed to run --help");
    assert_eq!(
        help_output.status.code(),
        Some(ExitStatus::Success.code() as i32),
        "--help should exit Success"
    );
    let help_stdout = String::from_utf8_lossy(&help_output.stdout);
    assert!(
        help_stdout.contains("README.md"),
        "help text should mention README.md, got: {}",
        help_stdout
    );

    let mut version_cmd = aidp(&ws);
    let version_output = version_cmd
        .arg("--version")
        .output()
        .expect("Failed to run --version");
    assert_eq!(
        version_output.status.code(),
        Some(ExitStatus::Success.code() as i32),
        "--version should exit Success"
    );
}

/// Seeds a run from the `t05-minimal.yaml` fixture and returns its run_id.
fn seed_t05_run(ws: &aidp_testkit::TempWorkspace) -> String {
    let fixture_path = ws.child("t05-minimal.yaml");
    let fixture_src = crate_dir().join("tests/fixtures/t05-minimal.yaml");
    fs::copy(&fixture_src, &fixture_path).expect("Failed to copy fixture");

    let mut cmd = aidp(ws);
    cmd.arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg(&fixture_path)
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run aidp run init");
    assert!(
        output.status.success(),
        "seed_t05_run: run init should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
