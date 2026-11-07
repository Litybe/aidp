//! End-to-end tests for the gate lifecycle: run start, gate approve/abort/pending,
//! and run record-resume.
//!
//! This is a CHARACTERIZATION test file for LOC-18 that freezes the current behavior
//! of the `aidp` gate system exactly as it is today, including three real anomalies:
//! - gate approve does not validate gate names (main.rs:611 GateOp enum vs main.rs:1022 validate)
//! - gate abort leaves the gate approved (run.rs:876 filter keeps both records)
//! - gate pending on an already-approved gate still reports success (main.rs:1346 prints unconditionally)
//!
//! All eleven scenarios run in under 30 seconds; no agent is ever launched.

use crate::common;
use aidp_testkit::TempWorkspace;
use std::fs;

/// Helper to start a run and parse the JSON envelope for gates/auto_approved_gates.
fn start_study_run(ws: &TempWorkspace) -> (String, serde_json::Value) {
    let mut cmd = common::aidp_with_fixture_skill(ws);
    cmd.arg("run")
        .arg("start")
        .arg("--pipeline")
        .arg("study")
        .arg("--input")
        .arg("topic=x")
        .arg("--workspace-root")
        .arg(".")
        .arg("--no-vault");

    let output = cmd.output().expect("run start should execute");
    assert!(output.status.success(), "run start should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let env: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("run start should output valid JSON");

    let run_id = env["run_id"]
        .as_str()
        .expect("run_id should be present")
        .to_string();

    (run_id, env)
}

/// Scenario 1: run start with study template produces the correct envelope.
#[test]
fn test_run_start_produces_gates_envelope() {
    let ws = common::workspace();
    let (run_id, env) = start_study_run(&ws);

    // Check the envelope keys
    assert!(env["run_id"].is_string(), "run_id should be string");
    assert!(env["run_dir"].is_string(), "run_dir should be string");
    assert!(
        env["vault"].is_null(),
        "vault should be null (no-vault mode)"
    );
    assert!(env["gates"].is_array(), "gates should be array");
    assert!(
        env["auto_approved_gates"].is_array(),
        "auto_approved_gates should be array"
    );
    assert!(
        env["awaiting_plan"].is_boolean(),
        "awaiting_plan should be boolean"
    );

    // `study.yaml` opens with a `type: pipeline` stage (`knowledge`, `uses:
    // knowledge-read`). Since LOC-48 a composite only forces its own pass when
    // crossing it would exceed `settings.max_stages_per_pass`; `knowledge-read`
    // is small enough to be merged, so this first pass runs straight through
    // to `synthesize` and reports its gate.
    assert_eq!(
        env["awaiting_composite"].as_bool(),
        Some(false),
        "study's small `knowledge` composite is merged into the first pass (LOC-48)"
    );
    let gates = env["gates"].as_array().expect("gates should be array");
    assert!(
        gates.iter().any(|g| g == "synthesize"),
        "synthesize's gate is reachable in the first pass: {gates:?}"
    );

    let auto_approved = env["auto_approved_gates"]
        .as_array()
        .expect("auto_approved_gates should be array");
    assert_eq!(
        auto_approved.len(),
        0,
        "no gates should be auto-approved yet"
    );

    assert_eq!(
        env["awaiting_plan"].as_bool(),
        Some(false),
        "awaiting_plan should be false"
    );

    // Check that run-state.json exists
    // The run_dir path is relative to the workspace, so make it absolute
    let run_dir_str = env["run_dir"].as_str().expect("run_dir");
    let run_state_path = ws.path().join(run_dir_str).join("run-state.json");
    assert!(run_state_path.exists(), "run-state.json should exist");

    // Cross the `knowledge` boundary and re-emit: `synthesize`'s gate guard
    // should now appear in the compiled script, proving the gate really is
    // there once compilation is no longer cut short by the composite.
    let mut cross_cmd = common::aidp_with_fixture_skill(&ws);
    cross_cmd
        .arg("run")
        .arg("record-resume")
        .arg("--run")
        .arg(&run_id)
        .arg("--crossed-boundary")
        .arg("knowledge")
        .arg("--workspace-root")
        .arg(".");
    assert!(
        cross_cmd
            .output()
            .expect("record-resume should execute")
            .status
            .success(),
        "record-resume --crossed-boundary should succeed"
    );

    let mut emit_cmd = common::aidp_with_fixture_skill(&ws);
    emit_cmd
        .arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".");
    assert!(
        emit_cmd
            .output()
            .expect("workflow emit should execute")
            .status
            .success(),
        "workflow emit should succeed after crossing the boundary"
    );

    let workflow_js_path = ws.path().join(run_dir_str).join("workflow.js");
    let script_content =
        fs::read_to_string(&workflow_js_path).expect("workflow.js should be readable");
    assert!(
        script_content.contains("gate: \"synthesize\""),
        "synthesize's gate guard should appear once the composite boundary is crossed"
    );
}

/// Scenario 2: gate approve produces the correct envelope with camelCase keys.
///
/// Note: `scriptPath` and `resumeFromRunId` are camelCase ON PURPOSE because they are
/// the argument names imposed by Claude Code's `Workflow` tool (main.rs :1312-1313),
/// unlike every other envelope in that file which is snake_case. This is the one comment
/// the production code is missing.
#[test]
fn test_gate_approve_workflow_envelope() {
    let ws = common::workspace();
    let (run_id, _env) = start_study_run(&ws);

    let mut cmd = common::aidp_with_fixture_skill(&ws);
    cmd.arg("gate")
        .arg("approve")
        .arg("--run")
        .arg(&run_id)
        .arg("--gate")
        .arg("synthesize")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("gate approve should execute");
    assert!(output.status.success(), "gate approve should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let approve_env: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("gate approve should output valid JSON");

    // Check key set is exactly {args, resumeFromRunId, scriptPath}
    let keys: Vec<&str> = approve_env
        .as_object()
        .expect("should be object")
        .keys()
        .map(|s| s.as_str())
        .collect();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    assert_eq!(
        sorted_keys,
        vec!["args", "resumeFromRunId", "scriptPath"],
        "envelope keys should be exactly {{args, resumeFromRunId, scriptPath}}"
    );

    // Check args.approved_gates
    let approved_gates = &approve_env["args"]["approved_gates"];
    assert!(approved_gates.is_array(), "approved_gates should be array");
    let gates_vec = approved_gates.as_array().expect("approved_gates array");
    assert_eq!(gates_vec.len(), 1, "should have one approved gate");
    assert_eq!(
        gates_vec[0].as_str(),
        Some("synthesize"),
        "approved gate should be synthesize"
    );

    // Check resumeFromRunId is null (no workflow_run_id passed)
    assert!(
        approve_env["resumeFromRunId"].is_null(),
        "resumeFromRunId should be null when no --workflow-run-id passed"
    );

    // Check scriptPath ends with workflow.js
    let script_path_str = approve_env["scriptPath"]
        .as_str()
        .expect("scriptPath should be string");
    assert!(
        script_path_str.ends_with("workflow.js"),
        "scriptPath should end with workflow.js, got: {}",
        script_path_str
    );
    let script_path = ws.path().join(script_path_str);
    assert!(script_path.exists(), "scriptPath should exist");
}

/// Scenario 3: After approval, workflow.js contains __approvedBaked and __approved.
#[test]
fn test_workflow_js_contains_approved_baked() {
    let ws = common::workspace();
    let (run_id, _env) = start_study_run(&ws);

    // Approve the gate
    let mut cmd = common::aidp_with_fixture_skill(&ws);
    cmd.arg("gate")
        .arg("approve")
        .arg("--run")
        .arg(&run_id)
        .arg("--gate")
        .arg("synthesize")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("gate approve should execute");
    assert!(output.status.success(), "gate approve should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let approve_env: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("gate approve should output valid JSON");

    let script_path_str = approve_env["scriptPath"]
        .as_str()
        .expect("scriptPath should be string");

    // Read workflow.js (convert relative path to absolute)
    let script_path = ws.path().join(script_path_str);
    let script_content = fs::read_to_string(&script_path).expect("workflow.js should be readable");

    // Check for __approvedBaked (workflow_emit.rs :2154-2159)
    assert!(
        script_content.contains("const __approvedBaked = [\"synthesize\"];"),
        "workflow.js should contain const __approvedBaked = [\"synthesize\"];"
    );

    // Check for __approved union
    assert!(
        script_content.contains("const __approved = Array.from(new Set([...__approvedBaked, ...__approvedArgs]));"),
        "workflow.js should contain const __approved = Array.from(new Set([...__approvedBaked, ...__approvedArgs]));"
    );
}

/// `gate approve` refuses an unknown gate name and never bakes it into the emitted script.
/// A known gate is approved first to prove the refusal is specific to the unknown name.
#[test]
fn test_approve_unknown_gate_is_refused() {
    let ws = common::workspace();
    let (run_id, _env) = start_study_run(&ws);

    {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("gate")
            .arg("approve")
            .arg("--run")
            .arg(&run_id)
            .arg("--gate")
            .arg("synthesize")
            .arg("--workspace-root")
            .arg(".");

        let output = cmd.output().expect("gate approve should execute");
        assert!(output.status.success(), "first approval should succeed");
    }

    {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("gate")
            .arg("approve")
            .arg("--run")
            .arg(&run_id)
            .arg("--gate")
            .arg("nosuchgate")
            .arg("--workspace-root")
            .arg(".");

        let output = cmd.output().expect("gate approve should execute");
        assert!(
            !output.status.success(),
            "approval of an unknown gate should be refused"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("unknown gate `nosuchgate`"), "{stderr}");
        assert!(stderr.contains("synthesize"), "{stderr}");
    }

    // Load workflow.js and verify both names are baked
    let run_state_path = ws.path().join(format!(
        ".aidp/pipelines/_no-ticket/{}/run-state.json",
        run_id
    ));
    let run_state_content =
        fs::read_to_string(&run_state_path).expect("run-state.json should exist");
    let run_state: serde_json::Value =
        serde_json::from_str(&run_state_content).expect("run-state.json should be valid JSON");

    let run_dir = run_state["run_dir"]
        .as_str()
        .expect("run_dir should be in state");
    let workflow_js_path = ws.path().join(run_dir).join("workflow.js");
    let script_content =
        fs::read_to_string(&workflow_js_path).expect("workflow.js should be readable");

    assert!(
        script_content.contains("synthesize") && !script_content.contains("nosuchgate"),
        "workflow.js should bake synthesize and never the refused nosuchgate"
    );
}

/// Scenario 5: gate abort prints literal stdout but does not re-emit workflow.js,
/// and the gate stays approved.
/// This is anomaly #2: `aidp_core::run::approved_gates` keeps only `decision == "approve"`
/// (run.rs:876) while abort pushes a second decision record without removing the first.
///
/// FIGE-A-DESSEIN: `aidp_core::run::approved_gates` filters gate records by decision=="approve"
/// (run.rs:872-880) while `gate abort` (main.rs:1319-1345) pushes a second GateDecision without
/// removing the approval. The filtering logic keeps both records: the approval gates the
/// emitted script, the abort record is ceremonial.
#[test]
fn test_gate_abort_leaves_gate_approved() {
    let ws = common::workspace();
    let (run_id, _env) = start_study_run(&ws);

    // Approve the gate
    let mut cmd = common::aidp_with_fixture_skill(&ws);
    cmd.arg("gate")
        .arg("approve")
        .arg("--run")
        .arg(&run_id)
        .arg("--gate")
        .arg("synthesize")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("gate approve should execute");
    assert!(output.status.success(), "approval should succeed");

    let approve_output = String::from_utf8_lossy(&output.stdout);
    let approve_env: serde_json::Value =
        serde_json::from_str(approve_output.trim()).expect("approval output should be valid JSON");
    let script_path_before = approve_env["scriptPath"]
        .as_str()
        .expect("scriptPath should be string")
        .to_string();
    let script_path_abs = ws.path().join(&script_path_before);
    let script_bytes_before = fs::read(&script_path_abs).expect("workflow.js should be readable");

    // Now abort the gate
    {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("gate")
            .arg("abort")
            .arg("--run")
            .arg(&run_id)
            .arg("--gate")
            .arg("synthesize")
            .arg("--workspace-root")
            .arg(".");

        let output = cmd.output().expect("gate abort should execute");
        assert!(output.status.success(), "abort should succeed");

        let abort_stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = abort_stdout.trim();
        assert_eq!(
            trimmed, r#"{"ok": true, "gate": "synthesize", "decision": "abort"}"#,
            "abort should print exact literal stdout"
        );
    }

    // Check that workflow.js is byte-for-byte identical (abort does not re-emit)
    let script_bytes_after = fs::read(&script_path_abs).expect("workflow.js should still exist");
    assert_eq!(
        script_bytes_before, script_bytes_after,
        "workflow.js should be unchanged after abort (abort does not re-emit)"
    );

    // Check that record-resume still reports the gate as approved
    {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("run")
            .arg("record-resume")
            .arg("--run")
            .arg(&run_id)
            .arg("--workspace-root")
            .arg(".");

        let output = cmd.output().expect("record-resume should execute");
        assert!(output.status.success(), "record-resume should succeed");

        let resume_stdout = String::from_utf8_lossy(&output.stdout);
        let resume_env: serde_json::Value = serde_json::from_str(resume_stdout.trim())
            .expect("record-resume output should be valid JSON");

        let approved = resume_env["approved_gates"]
            .as_array()
            .expect("approved_gates should be array");
        assert_eq!(approved.len(), 1, "should still have one approved gate");
        assert_eq!(
            approved[0].as_str(),
            Some("synthesize"),
            "synthesize should still be approved (abort does not remove it)"
        );
    }
}

/// Scenario 6: gate pending on an undecided run records the pending flag.
#[test]
fn test_gate_pending_on_undecided_run() {
    let ws = common::workspace();
    let (run_id, _env) = start_study_run(&ws);

    // Pend the gate
    let mut cmd = common::aidp_with_fixture_skill(&ws);
    cmd.arg("gate")
        .arg("pending")
        .arg("--run")
        .arg(&run_id)
        .arg("--gate")
        .arg("synthesize")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("gate pending should execute");
    assert!(output.status.success(), "pending should succeed");

    let pending_stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = pending_stdout.trim();
    assert_eq!(
        trimmed, r#"{"ok": true, "gate": "synthesize", "decision": "pending"}"#,
        "pending should print exact literal stdout"
    );

    // Check that record-resume reports pending_gate
    {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("run")
            .arg("record-resume")
            .arg("--run")
            .arg(&run_id)
            .arg("--workspace-root")
            .arg(".");

        let output = cmd.output().expect("record-resume should execute");
        assert!(output.status.success(), "record-resume should succeed");

        let resume_stdout = String::from_utf8_lossy(&output.stdout);
        let resume_env: serde_json::Value = serde_json::from_str(resume_stdout.trim())
            .expect("record-resume output should be valid JSON");

        assert_eq!(
            resume_env["pending_gate"].as_str(),
            Some("synthesize"),
            "pending_gate should be synthesize"
        );

        let approved = resume_env["approved_gates"]
            .as_array()
            .expect("approved_gates should be array");
        assert_eq!(approved.len(), 0, "should have no approved gates");
    }
}

/// Scenario 7: gate pending on an already-approved run still prints success,
/// but record-resume lets the approval win and clears the pending flag.
/// This is anomaly #3: the Pending arm prints its verdict unconditionally
/// (main.rs:1346-1356) without re-reading state, whereas record-resume lets
/// approval win (run.rs:857-866).
///
/// FIGE-A-DESSEIN: `record_resume` function (run.rs:857-866) checks if an approval
/// exists for the pending gate and clears the pending flag if so, letting the approval
/// win. The `Pending` arm of GateOp (main.rs:1346-1356) prints its decision verdict
/// unconditionally without re-reading state afterward, so it reports "pending" even
/// though record-resume will discard it.
#[test]
fn test_gate_pending_on_approved_gate() {
    let ws = common::workspace();
    let (run_id, _env) = start_study_run(&ws);

    // First approve the gate
    {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("gate")
            .arg("approve")
            .arg("--run")
            .arg(&run_id)
            .arg("--gate")
            .arg("synthesize")
            .arg("--workspace-root")
            .arg(".");

        let output = cmd.output().expect("gate approve should execute");
        assert!(output.status.success(), "approval should succeed");
    }

    // Now try to pend the same gate
    {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("gate")
            .arg("pending")
            .arg("--run")
            .arg(&run_id)
            .arg("--gate")
            .arg("synthesize")
            .arg("--workspace-root")
            .arg(".");

        let output = cmd.output().expect("gate pending should execute");
        assert!(output.status.success(), "pending should succeed");

        let pending_stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = pending_stdout.trim();
        assert_eq!(
            trimmed, r#"{"ok": true, "gate": "synthesize", "decision": "pending"}"#,
            "pending still prints its verdict (anomaly)"
        );
    }

    // Check that record-resume clears the pending flag
    {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("run")
            .arg("record-resume")
            .arg("--run")
            .arg(&run_id)
            .arg("--workspace-root")
            .arg(".");

        let output = cmd.output().expect("record-resume should execute");
        assert!(output.status.success(), "record-resume should succeed");

        let resume_stdout = String::from_utf8_lossy(&output.stdout);
        let resume_env: serde_json::Value = serde_json::from_str(resume_stdout.trim())
            .expect("record-resume output should be valid JSON");

        assert!(
            resume_env["pending_gate"].is_null(),
            "pending_gate should be null because approval wins"
        );

        let approved = resume_env["approved_gates"]
            .as_array()
            .expect("approved_gates should be array");
        assert_eq!(approved.len(), 1, "should still have the approval");
    }
}

/// Scenario 8: run start with --approve-gate '*' auto-approves all declared gates.
#[test]
fn test_run_start_approve_gate_wildcard() {
    let ws = common::workspace();

    let mut cmd = common::aidp_with_fixture_skill(&ws);
    cmd.arg("run")
        .arg("start")
        .arg("--pipeline")
        .arg("study")
        .arg("--input")
        .arg("topic=x")
        .arg("--workspace-root")
        .arg(".")
        .arg("--no-vault")
        .arg("--approve-gate")
        .arg("*");

    let output = cmd.output().expect("run start should execute");
    assert!(output.status.success(), "run start should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let env: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("run start should output valid JSON");

    let auto_approved = env["auto_approved_gates"]
        .as_array()
        .expect("auto_approved_gates should be array");
    assert_eq!(auto_approved.len(), 1, "should have one auto-approved gate");
    assert_eq!(
        auto_approved[0].as_str(),
        Some("synthesize"),
        "auto-approved gate should be synthesize"
    );

    let gates = env["gates"].as_array().expect("gates should be array");
    assert_eq!(
        gates.len(),
        0,
        "gates should be empty when all are auto-approved"
    );

    // Check that workflow.js already contains __approvedBaked
    let run_dir = env["run_dir"].as_str().expect("run_dir");
    let workflow_js_path = ws.path().join(run_dir).join("workflow.js");
    let script_content =
        fs::read_to_string(&workflow_js_path).expect("workflow.js should be readable");

    assert!(
        script_content.contains("const __approvedBaked = [\"synthesize\"];"),
        "workflow.js should contain baked synthesize gate"
    );
}

/// Scenario 9: run start with --approve-gate for unknown gate warns but succeeds.
/// Also verifies that AIDP_AUTO_APPROVE_GATES environment variable works as a fallback
/// and must be scrubbed from the child env in the harness.
#[test]
fn test_run_start_approve_gate_unknown_and_env_fallback() {
    // Part 1: --approve-gate with unknown gate
    {
        let ws = common::workspace();

        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("run")
            .arg("start")
            .arg("--pipeline")
            .arg("study")
            .arg("--input")
            .arg("topic=x")
            .arg("--workspace-root")
            .arg(".")
            .arg("--no-vault")
            .arg("--approve-gate")
            .arg("nosuchgate");

        let output = cmd.output().expect("run start should execute");
        assert!(output.status.success(), "run start should succeed");

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("no stage declares a gate by that name"),
            "stderr should contain warning about unknown gate"
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let env: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("run start should output valid JSON");

        let auto_approved = env["auto_approved_gates"]
            .as_array()
            .expect("auto_approved_gates should be array");
        assert_eq!(
            auto_approved.len(),
            0,
            "should not auto-approve unknown gate"
        );

        // The `knowledge` composite is merged into the first pass (LOC-48), so
        // `synthesize`'s gate is reached — and an unknown `--approve-gate`
        // must not have pre-approved it.
        let gates = env["gates"].as_array().expect("gates should be array");
        assert!(
            gates.iter().any(|g| g == "synthesize"),
            "synthesize should still await approval: {gates:?}"
        );
    }

    // Part 2: AIDP_AUTO_APPROVE_GATES environment variable fallback
    {
        let ws = common::workspace();

        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("run")
            .arg("start")
            .arg("--pipeline")
            .arg("study")
            .arg("--input")
            .arg("topic=x")
            .arg("--workspace-root")
            .arg(".")
            .arg("--no-vault")
            .env("AIDP_AUTO_APPROVE_GATES", "synthesize");

        let output = cmd.output().expect("run start should execute");
        assert!(output.status.success(), "run start should succeed");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let env: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("run start should output valid JSON");

        let auto_approved = env["auto_approved_gates"]
            .as_array()
            .expect("auto_approved_gates should be array");
        assert_eq!(
            auto_approved.len(),
            1,
            "should auto-approve from AIDP_AUTO_APPROVE_GATES"
        );
        assert_eq!(
            auto_approved[0].as_str(),
            Some("synthesize"),
            "auto-approved gate should be synthesize"
        );
    }
}

/// Scenario 10: run record-resume with workflow_run_id and approved_gate produces
/// a four-key envelope with snake_case keys, and is idempotent.
#[test]
fn test_run_record_resume_idempotent() {
    let ws = common::workspace();
    let (run_id, _env) = start_study_run(&ws);

    // Call record-resume twice with the same workflow_run_id and gate
    for _iteration in 0..2 {
        let mut cmd = common::aidp_with_fixture_skill(&ws);
        cmd.arg("run")
            .arg("record-resume")
            .arg("--run")
            .arg(&run_id)
            .arg("--workspace-root")
            .arg(".")
            .arg("--workflow-run-id")
            .arg("wf-123")
            .arg("--approved-gate")
            .arg("synthesize");

        let output = cmd.output().expect("record-resume should execute");
        assert!(output.status.success(), "record-resume should succeed");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let resume_env: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("record-resume output should be valid JSON");

        // Check key set is exactly {ok, workflow_run_id, approved_gates, pending_gate,
        // crossed_boundaries} — the last one added alongside `awaiting_composite`
        // (composite pass-boundary crossing), never populated by this scenario since
        // it only ever passes `--approved-gate`.
        let keys: Vec<&str> = resume_env
            .as_object()
            .expect("should be object")
            .keys()
            .map(|s| s.as_str())
            .collect();
        let mut sorted_keys = keys.clone();
        sorted_keys.sort();
        assert_eq!(
            sorted_keys,
            vec![
                "approved_gates",
                "crossed_boundaries",
                "ok",
                "pending_gate",
                "workflow_run_id"
            ],
            "envelope keys should be exactly {{ok, workflow_run_id, approved_gates, pending_gate, crossed_boundaries}}"
        );
        assert!(
            resume_env["crossed_boundaries"]
                .as_array()
                .expect("crossed_boundaries should be array")
                .is_empty(),
            "no composite boundary was crossed in this scenario"
        );

        // Check values
        assert_eq!(
            resume_env["workflow_run_id"].as_str(),
            Some("wf-123"),
            "workflow_run_id should be wf-123"
        );

        let approved = resume_env["approved_gates"]
            .as_array()
            .expect("approved_gates should be array");
        assert_eq!(approved.len(), 1, "should have one approved gate");
        assert_eq!(
            approved[0].as_str(),
            Some("synthesize"),
            "approved gate should be synthesize"
        );

        assert!(
            resume_env["pending_gate"].is_null(),
            "pending_gate should be null"
        );
    }
}

/// Scenario 11: gate approve on a nonexistent run exits 1 and mentions run-state.json in stderr.
#[test]
fn test_gate_approve_nonexistent_run_fails() {
    let ws = common::workspace();

    let mut cmd = common::aidp_with_fixture_skill(&ws);
    cmd.arg("gate")
        .arg("approve")
        .arg("--run")
        .arg("19990101-000000-dead")
        .arg("--gate")
        .arg("synthesize")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("gate approve should execute");
    assert!(!output.status.success(), "gate approve should fail");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "stdout should be empty on failure"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("run-state.json"),
        "stderr should mention run-state.json"
    );
}
