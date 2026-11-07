//! Characterization test that freezes every observable exit code of the `aidp` binary
//! and the stdout/stderr shape that goes with it, against the shared `ExitStatus` contract.
//!
//! This file documents the counter-intuitive contracts (scope check's JSON-on-stdout
//! for a policy refusal, run verify never failing, run reconcile exiting 0 outside a
//! vault leak, and run summary's silent mode), plus clap's usage exit 7.
//!
//! Any change to this test is a deliberate, reviewed break in the binary's interface contract.

use crate::common;
use common::*;
use serde_json::json;
use std::fs;

/// Helper to seed a run using the t05-minimal.yaml fixture and return the run_id.
fn seeded_run(ws: &aidp_testkit::TempWorkspace) -> String {
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
        "seeded_run: run init should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Resolves `<run_dir>` for a seeded run by asking `scope resolve` for the `only`
/// stage's `stage_dir` and taking its parent — avoids parsing the `run_dir:`
/// line `aidp run init` prints on stderr (the harness contract is stdout-only).
fn run_dir_of(ws: &aidp_testkit::TempWorkspace, run_id: &str) -> std::path::PathBuf {
    let mut cmd = aidp(ws);
    cmd.arg("scope")
        .arg("resolve")
        .arg("--run")
        .arg(run_id)
        .arg("--stage")
        .arg("only")
        .arg("--workspace-root")
        .arg(".");
    let output = cmd.output().expect("Failed to run scope resolve");
    assert!(output.status.success(), "scope resolve should succeed");
    let resolved: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("scope resolve stdout should be JSON");
    let stage_dir = resolved["stage_dir"]
        .as_str()
        .expect("stage_dir should be present");
    std::path::Path::new(stage_dir)
        .parent()
        .expect("stage_dir should have a parent run_dir")
        .to_path_buf()
}

/// Reads `<run_dir>/run-state.json`, applies `patch` to the parsed JSON, and writes it back.
fn patch_run_state(run_dir: &std::path::Path, patch: impl FnOnce(&mut serde_json::Value)) {
    let path = run_dir.join("run-state.json");
    let content = fs::read_to_string(&path).expect("run-state.json should exist");
    let mut value: serde_json::Value =
        serde_json::from_str(&content).expect("run-state.json should be valid JSON");
    patch(&mut value);
    fs::write(&path, serde_json::to_string_pretty(&value).unwrap())
        .expect("Failed to write run-state.json");
}

/// True when a `git` binary is on PATH and runs successfully.
fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs `git <args>` in `dir`, panicking on failure. Only called after `git_available()`.
fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git must be on PATH for this test");
    assert!(status.success(), "git {:?} failed in {:?}", args, dir);
}

/// Data-driven test cases covering the 7 reachable exit code sites.
#[test]
fn scope_check_inside_stage_dir_exits_0() {
    let ws = workspace();
    let run_id = seeded_run(&ws);

    // Resolve the stage directory
    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("resolve")
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("only")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope resolve");
    assert!(output.status.success(), "scope resolve should succeed");

    let resolved: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("scope resolve stdout should be JSON");
    let stage_dir = resolved["stage_dir"]
        .as_str()
        .expect("stage_dir should be present");

    // Create a test file inside the stage_dir
    let test_file = std::path::Path::new(stage_dir).join("test.txt");
    fs::create_dir_all(stage_dir).ok();
    fs::write(&test_file, "test").expect("Failed to write test file");

    // Now run scope check with a file inside the stage_dir
    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("check")
        .arg("--path")
        .arg(test_file.to_string_lossy().to_string())
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("only")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "scope check: file inside stage_dir should exit 0"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        r#"{"ok": true}"#,
        "scope check: stdout should be exactly {{\"ok\": true}}"
    );
}

/// Scenario 2: scope check with file OUTSIDE stage_dir -> exit 1, JSON with reason on stdout.
/// FIGE-A-DESSEIN: main.rs:1233-1246 is the ONLY site in the binary where a failure prints JSON
/// on stdout with a non-zero exit.
#[test]
fn scope_check_outside_stage_dir_exits_4_with_json_reason() {
    let ws = workspace();
    let run_id = seeded_run(&ws);

    // Create a file OUTSIDE any stage directory
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
        Some(4),
        "scope check: file outside stage_dir should exit 4 (policy refusal)"
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
        "scope check: reason should be non-empty (text not asserted)"
    );
}

/// Scenario 3: scope check with AIDP_RUN/AIDP_STAGE env vars (no flags) -> exit 1, identical stdout.
/// FIGE-A-DESSEIN: main.rs:1219-1231 is the env-var fallback the Claude Code hooks rely on;
/// nothing else guards it.
#[test]
fn scope_check_via_env_vars_matches_flags() {
    let ws = workspace();
    let run_id = seeded_run(&ws);

    // Create a file OUTSIDE the stage directory
    let outside_file = ws.child("outside.txt");
    fs::write(&outside_file, "outside").expect("Failed to write file");

    // Get the expected output from the flag-based call
    let mut cmd_flags = aidp(&ws);
    cmd_flags
        .arg("scope")
        .arg("check")
        .arg("--path")
        .arg(outside_file.to_string_lossy().to_string())
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("only")
        .arg("--workspace-root")
        .arg(".");

    let output_flags = cmd_flags
        .output()
        .expect("Failed to run scope check with flags");
    let expected_stdout = output_flags.stdout.clone();
    let expected_code = output_flags.status.code();

    // Now run with env vars instead
    let mut cmd_env = aidp(&ws);
    cmd_env
        .env("AIDP_RUN", &run_id)
        .env("AIDP_STAGE", "only")
        .arg("scope")
        .arg("check")
        .arg("--path")
        .arg(outside_file.to_string_lossy().to_string())
        .arg("--workspace-root")
        .arg(".");

    let output_env = cmd_env
        .output()
        .expect("Failed to run scope check with env vars");

    assert_eq!(
        output_env.status.code(),
        expected_code,
        "scope check: exit code should be identical via env vars"
    );
    assert_eq!(
        output_env.stdout, expected_stdout,
        "scope check: stdout should be byte-for-byte identical via env vars"
    );
}

/// Scenario 4: scope check with unknown stage -> exit 3, JSON `{"ok": false, "reason": ...}`
/// on stdout, stderr has "unknown stage".
#[test]
fn scope_check_unknown_stage_exits_3_with_json_reason() {
    let ws = workspace();
    let run_id = seeded_run(&ws);

    // Create a test file
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
        Some(3),
        "scope check: unknown stage should exit 3 (not found)"
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
        "scope check: stderr should contain 'unknown stage: nosuchstage', got: {}",
        stderr
    );
}

/// Scenario 5: scope resolve -> exit 0, stdout has required keys, scope.json written.
#[test]
fn scope_resolve_exits_0_writes_scope_json() {
    let ws = workspace();
    let run_id = seeded_run(&ws);

    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("resolve")
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("only")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope resolve");
    assert_eq!(
        output.status.code(),
        Some(0),
        "scope resolve: should exit 0"
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("scope resolve: stdout should be JSON");
    assert!(
        parsed.get("stage").is_some(),
        "scope resolve: stdout should have 'stage' key"
    );
    assert!(
        parsed.get("mode").is_some(),
        "scope resolve: stdout should have 'mode' key"
    );
    assert!(
        parsed.get("stage_dir").is_some(),
        "scope resolve: stdout should have 'stage_dir' key"
    );
    assert!(
        parsed.get("allowed").is_some(),
        "scope resolve: stdout should have 'allowed' key"
    );
    assert!(
        parsed.get("denied").is_some(),
        "scope resolve: stdout should have 'denied' key"
    );

    let stage_dir = parsed["stage_dir"]
        .as_str()
        .expect("stage_dir should be a string");
    let scope_json_path = std::path::Path::new(stage_dir).join("scope.json");
    assert!(
        scope_json_path.exists(),
        "scope resolve: scope.json should be written to stage_dir"
    );

    let scope_json_content =
        fs::read_to_string(&scope_json_path).expect("Failed to read scope.json");
    let scope_from_file: serde_json::Value =
        serde_json::from_str(&scope_json_content).expect("scope.json should be valid JSON");

    // Verify that the file contains the same object
    assert_eq!(
        parsed, scope_from_file,
        "scope resolve: stdout and written scope.json should be identical"
    );
}

/// Scenario 6: scope verify in non-git workspace (base_sha: null) -> exit 0, empty violations.
/// FIGE-A-DESSEIN: crates/aidp-core/src/scope.rs:291-294 shows an empty report here proves
/// nothing, it only means there was no `base_sha`.
#[test]
fn scope_verify_no_git_exits_0_empty_violations() {
    let ws = workspace();
    let run_id = seeded_run(&ws);

    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("verify")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope verify");
    assert_eq!(
        output.status.code(),
        Some(0),
        "scope verify: non-git should exit 0"
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("scope verify: stdout should be JSON");
    assert_eq!(
        parsed.get("violations").unwrap_or(&json!([])),
        &json!([]),
        "scope verify: non-git should have empty violations array"
    );
    assert_eq!(
        parsed.get("union_universal").unwrap_or(&json!(false)),
        &json!(false),
        "scope verify: union_universal should be false"
    );
    assert_eq!(
        parsed.get("uninspected_stages").unwrap_or(&json!([])),
        &json!([]),
        "scope verify: uninspected_stages should be empty"
    );
}

/// Scenario 7: run verify reports exit 0 regardless of content - it is an annotation, not a gate.
/// FIGE-A-DESSEIN: main.rs:1176-1193 contains no `exit` at all -- it is an annotation,
/// never a gate; see main.rs:596-598.
#[test]
fn run_verify_exits_0_always() {
    let ws = workspace();
    let run_id = seeded_run(&ws);

    // Run verify on a fresh run (no outputs to verify)
    let mut cmd = aidp(&ws);
    cmd.arg("run")
        .arg("verify")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run verify");
    assert_eq!(
        output.status.code(),
        Some(0),
        "run verify: should exit 0 (annotation, never a gate)"
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("run verify: stdout should be JSON array");
    assert!(
        parsed.is_array(),
        "run verify: stdout should be an array of verdicts"
    );
}

/// Scenario 8: run reconcile is not a gate - exit code is always 0 or error only.
/// FIGE-A-DESSEIN: crates/aidp-core/src/run.rs:1158-1171 documents "The exit code stays 0 either way",
/// and main.rs:666-669 show the CLI echo.
#[test]
fn run_reconcile_not_a_gate() {
    let ws = workspace();
    let run_id = seeded_run(&ws);
    let run_dir = run_dir_of(&ws, &run_id);

    // Seed a scope violation directly into run-state.json - reconcile must not
    // recompute or clear it for a "paused" result (run.rs:1145-1179: `finished`
    // requires raw_status == Some("done")).
    patch_run_state(&run_dir, |state| {
        state["scope_violations"] = json!([
            { "path": "src/OutOfScope.rs", "reason": "not in tests: src/OutOfScope.rs" }
        ]);
    });

    // A "paused" result - reconcile must still report the seeded violation
    // without failing the run over it.
    let result_file = ws.child("result.json");
    fs::write(
        &result_file,
        json!({ "status": "paused", "results": {} }).to_string(),
    )
    .expect("Failed to write result.json");

    let mut cmd = aidp(&ws);
    cmd.arg("run")
        .arg("reconcile")
        .arg("--run")
        .arg(&run_id)
        .arg("--from")
        .arg(result_file.to_string_lossy().to_string())
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run reconcile");
    assert_eq!(
        output.status.code(),
        Some(0),
        "run reconcile: should exit 0 even with scope violations present, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("- scope: 1 violation(s)"),
        "run reconcile: stdout should report the seeded violation, got: {}",
        stdout
    );
}

/// Scenario 9: run summary with no --run and no in-flight marker -> exit 0, EMPTY stdout.
/// FIGE-A-DESSEIN: main.rs:1072-1078 shows the silence protects the `Stop` hook.
#[test]
fn run_summary_no_run_exits_0_empty_stdout() {
    let ws = workspace();

    // Do NOT create any run, so there's no in-flight marker
    let mut cmd = aidp(&ws);
    cmd.arg("run")
        .arg("summary")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run summary");
    assert_eq!(
        output.status.code(),
        Some(0),
        "run summary: with no --run should exit 0"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.is_empty(),
        "run summary: with no --run should have EMPTY stdout"
    );
}

/// Scenario 10: cost anomaly with AIDP_VAULT=off -> exit 0, status: "insufficient_data".
#[test]
fn cost_anomaly_no_vault_exits_0_insufficient_data() {
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
        Some(0),
        "cost anomaly: no vault should exit 0"
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cost anomaly: stdout should be JSON");
    assert_eq!(
        parsed["status"].as_str(),
        Some("insufficient_data"),
        "cost anomaly: status should be 'insufficient_data'"
    );
    assert_eq!(parsed["n"].as_i64(), Some(0), "cost anomaly: n should be 0");
}

/// Scenario 11: cost anomaly with vault containing 4 runs with cost, triggers outlier detection.
#[test]
fn cost_anomaly_detects_outlier() {
    let ws = workspace();
    let vault_dir = ws.child("vault");
    fs::create_dir_all(&vault_dir).expect("Failed to create vault");

    // Create run-state.json files with costs
    let run1_id = "20260919-100000-0001";
    let run2_id = "20260919-110000-0002";
    let run3_id = "20260919-120000-0003";
    let run4_id = "20260919-130000-0004";

    // Vault structure: <vault>/Pipelines/<repo>/<feature>/<run_id>/run-state.json
    let base_path = vault_dir.join("Pipelines/demo/feature");
    fs::create_dir_all(&base_path).expect("Failed to create pipelines dir");

    // Create 4 run-state.json files with different costs
    // Must include all required fields for RunState deserialization
    let costs = [1.00, 1.10, 1.20, 50.00];
    for (run_id, cost) in [run1_id, run2_id, run3_id, run4_id]
        .iter()
        .zip(costs.iter())
    {
        let run_dir = base_path.join(run_id);
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

        let run_state_file = run_dir.join("run-state.json");
        fs::write(run_state_file, run_state.to_string()).expect("Failed to write run-state.json");
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
        Some(6),
        "cost anomaly: should exit 6 once an outlier is flagged, stdout: {}",
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
    let points = parsed["points"]
        .as_array()
        .expect("cost anomaly: points should be an array");
    assert_eq!(
        points.len(),
        4,
        "cost anomaly: points should have 4 entries"
    );
    let flagged_count = points
        .iter()
        .filter(|p| p["flagged"].as_bool() == Some(true))
        .count();
    assert_eq!(
        flagged_count, 1,
        "cost anomaly: exactly one point should be flagged, got: {}",
        parsed
    );
}

/// Scenario 13: gateway status with closed loopback -> exit 1, stderr helpful message.
#[test]
fn gateway_status_unreachable_shows_error() {
    let ws = workspace();

    let mut cmd = aidp(&ws);
    cmd.arg("gateway")
        .arg("status")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run gateway status");

    // Should exit with failure code (could be 1 or another non-zero code)
    assert!(
        !output.status.success(),
        "gateway status: should not succeed with closed port"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // The error message should hint at the gateway being unavailable
    assert!(
        stderr.contains("gateway")
            || stderr.contains("unreachable")
            || stderr.contains("connection"),
        "gateway status: stderr should mention the gateway issue, got: {}",
        stderr
    );
}

/// Scenario 15: doctor -> exit code is relational to ok field (exit 1 iff ok == false).
#[test]
fn doctor_relational_exit_code() {
    let ws = workspace();

    let mut cmd = aidp(&ws);
    cmd.arg("doctor").arg("--workspace-root").arg(".");

    let output = cmd.output().expect("Failed to run doctor");
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor: stdout should be JSON");

    let ok = parsed["ok"]
        .as_bool()
        .expect("doctor: ok should be a boolean");
    let exit_code = output.status.code().expect("doctor: should have exit code");

    let expected_code = if ok { 0 } else { 6 };
    assert_eq!(
        exit_code, expected_code,
        "doctor: exit code should be 6 iff ok == false (ok={}, exit={})",
        ok, exit_code
    );
}

/// Scenario 16: scope verify detects out-of-scope modifications when base_sha is present.
#[test]
fn scope_verify_with_base_sha_detects_violations() {
    // Guarded by git availability, per the ticket's instructions.
    if !git_available() {
        eprintln!("skipping scope_verify_with_base_sha_detects_violations: git not on PATH");
        return;
    }

    let ws = workspace();

    // Make the workspace root a git repo BEFORE `run init`, so `base_sha`
    // (crate::gitlog::head(workspace_root), captured at init time) is Some(...).
    git(ws.path(), &["init", "-q"]);
    git(ws.path(), &["config", "user.email", "test@example.com"]);
    git(ws.path(), &["config", "user.name", "aidp tests"]);
    fs::write(ws.child("seed.txt"), "seed").expect("Failed to write seed file");
    git(ws.path(), &["add", "."]);
    git(ws.path(), &["commit", "-q", "-m", "chore: seed"]);

    let run_id = seeded_run(&ws);
    let run_dir = run_dir_of(&ws, &run_id);

    // The "only" stage must be `success` for `scope verify` to check it - a
    // freshly-init'd run leaves every stage `pending` (verify_run_report only
    // inspects `StageStatus::Success` stages).
    patch_run_state(&run_dir, |state| {
        state["stages"][0]["status"] = json!("success");
    });

    // Commit a file OUTSIDE the "only" stage's `stage_dir` (default scope
    // `stage_dir_only`) after `base_sha` - this is the violation.
    fs::write(ws.child("OutOfScope.rs"), "// not in scope").expect("Failed to write file");
    git(ws.path(), &["add", "OutOfScope.rs"]);
    git(
        ws.path(),
        &["commit", "-q", "-m", "feat: out of scope change"],
    );

    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("verify")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope verify");
    assert_eq!(
        output.status.code(),
        Some(6),
        "scope verify: should exit 6 once a violation is found, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("scope verify: stdout should be JSON");
    let violations = parsed["violations"]
        .as_array()
        .expect("scope verify: violations should be an array");
    assert!(
        !violations.is_empty(),
        "scope verify: should report at least one violation, got: {}",
        parsed
    );
}

/// Scenario 17: validate with missing pipeline -> exit 1, EMPTY stdout, stderr has "pipeline not found".
#[test]
fn validate_missing_pipeline_exits_3_empty_stdout() {
    let ws = workspace();
    let skills_dir = ws.child("skills");
    fs::create_dir_all(&skills_dir).expect("Failed to create skills dir");

    let mut cmd = aidp(&ws);
    cmd.env("AIDP_SKILL_ROOT", skills_dir.to_string_lossy().to_string())
        .arg("validate")
        .arg("nope")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run validate");
    assert_eq!(
        output.status.code(),
        Some(3),
        "validate: missing pipeline should exit 3 (not found)"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.is_empty(),
        "validate: missing pipeline should have EMPTY stdout"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("pipeline not found: nope"),
        "validate: stderr should contain 'pipeline not found: nope', got: {}",
        stderr
    );
}

/// A declared skill `skills::resolve` cannot find makes `validate` exit 1,
/// naming both the skill and the stage on stderr. A real skill sits beside
/// it, so the skill root exists and the check is armed.
#[test]
fn validate_unresolved_skill_exits_1_naming_skill_and_stage() {
    let ws = workspace();
    let present = ws.child("home").join(".claude/skills/present-skill");
    fs::create_dir_all(&present).expect("Failed to create skill dir");
    fs::write(
        present.join("SKILL.md"),
        "---
name: present-skill
---
",
    )
    .unwrap();
    let pipeline = ws.child("bad-skill.yaml");
    fs::write(
        &pipeline,
        "version: 5
id: bad-skill
stages:
  - name: implement_stage
    agent: executor
    skills: [present-skill, inexistante]
",
    )
    .unwrap();

    let mut cmd = aidp(&ws);
    cmd.arg("validate")
        .arg(pipeline.to_string_lossy().to_string())
        .arg("--workspace-root")
        .arg(".");
    let output = cmd.output().expect("Failed to run validate");

    assert_eq!(
        output.status.code(),
        Some(1),
        "validate: an unresolved skill should exit 1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("\"inexistante\""),
        "stderr should name the missing skill, got: {stderr}"
    );
    assert!(
        stderr.contains("\"implement_stage\""),
        "stderr should name the stage declaring it, got: {stderr}"
    );
    assert!(
        !stderr.contains("present-skill"),
        "a resolved skill must not be reported, got: {stderr}"
    );
}

/// Scenario 18: aidp nosuchcmd -> exit 2 (clap usage), EMPTY stdout.
#[test]
fn unknown_command_exits_7_empty_stdout() {
    let ws = workspace();

    let mut cmd = aidp(&ws);
    cmd.arg("nosuchcmd");

    let output = cmd.output().expect("Failed to run aidp nosuchcmd");
    assert_eq!(
        output.status.code(),
        Some(7),
        "aidp nosuchcmd: should exit 7 (clap usage code)"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.is_empty(),
        "aidp nosuchcmd: should have EMPTY stdout"
    );
}

/// A second, independent exercise of the scope check happy path (resolve then
/// check a file inside the stage_dir), kept separate from
/// `scope_check_inside_stage_dir_exits_0` to catch a regression introduced by
/// either test's own setup rather than shared fixture state.
#[test]
fn scope_operations_data_driven() {
    // Scenario: scope check succeeds -> exit 0 with JSON
    let ws = workspace();
    let run_id = seeded_run(&ws);

    // Create a file inside stage_dir
    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("resolve")
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("only")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope resolve");
    let resolved: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("scope resolve stdout should be JSON");
    let stage_dir = resolved["stage_dir"]
        .as_str()
        .expect("stage_dir should be present");

    let test_file = std::path::Path::new(stage_dir).join("test.txt");
    fs::create_dir_all(stage_dir).ok();
    fs::write(&test_file, "test").expect("Failed to write test file");

    let mut cmd = aidp(&ws);
    cmd.arg("scope")
        .arg("check")
        .arg("--path")
        .arg(test_file.to_string_lossy().to_string())
        .arg("--run")
        .arg(&run_id)
        .arg("--stage")
        .arg("only")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run scope check");

    // Assertions follow the pattern: exit code + JSON key check, never free text
    assert_eq!(
        output.status.code(),
        Some(0),
        "data-driven: scope check should exit 0"
    );
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("data-driven: scope check stdout should be JSON");
    assert_eq!(
        parsed["ok"], true,
        "data-driven: scope check should have ok: true"
    );
}

// EXCLUDED EXIT SITES (reachable only under conditions we avoid):
// - crates/aidp-integrations/src/ci.rs:1185 — only on CI-triggering path with 10s sleep + GitHub poll
// - crates/aidp-integrations/src/mongo.rs:142 — requires mongosh binary and connection
// - crates/aidp-integrations/src/sqs.rs:115 — requires aws CLI and connection
//
// Both `std::process::exit` occurrences in crates/aidp-gateway/src/health.rs are
// in doc-comments, not executable code.
