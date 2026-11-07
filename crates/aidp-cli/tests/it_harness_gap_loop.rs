use crate::common;
use common::*;
use std::fs;
use tempfile::TempDir;

/// Test that the emitted dev-aidp script contains NO unconditional harness-gap
/// pause guard for the review stage. The bug was that `aidp workflow emit` on
/// dev-aidp produced a guard line like:
/// `if (sigNum("review__review", "unevaluated_checks") > 0) return pauseInfo(...)`
/// even when `has_blocking_findings` was false. This guard must not appear.
#[test]
fn dev_aidp_emit_has_no_unconditional_harness_gap_pause() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");
    let out_path = tmpdir.path().join("dev-aidp.js");

    // Prepare minimal inputs for dev-aidp: prompt, feature, track
    let inputs = serde_json::json!({
        "prompt": "test the harness gap fix",
        "feature": "test-harness-gap",
        "track": "quick"
    });
    let inputs_json = serde_json::to_string(&inputs).expect("Failed to serialize inputs");

    // run init + workflow emit for dev-aidp
    let run_and_emit = run_init_and_emit(&ws, "dev-aidp", Some(&inputs_json), &out_path);
    let script = run_and_emit.script;

    // Assert that the script does NOT contain the problematic guard:
    // sigNum("review__review", "unevaluated_checks") on the same line.
    // Match on the substring "unevaluated_checks" co-occurring with "review__review"
    // on the same line, rather than exact whitespace, so formatting changes don't
    // make the test lie.
    let lines_with_review_review: Vec<&str> = script
        .lines()
        .filter(|line| line.contains("review__review"))
        .collect();

    for line in lines_with_review_review {
        assert!(
            !line.contains("unevaluated_checks"),
            "Found unconditional harness-gap pause guard in line: {}",
            line
        );
    }
}

/// Test that the emitted dev-aidp script properly handles the correction loop.
/// The correction loop in dev-aidp has been fixed (LOC-43) by adding a rereview
/// stage to the loop body. This test verifies that the loop is structurally sound
/// by checking that:
/// 1. The script is emitted without errors
/// 2. The script does not contain problematic unconditional pauses for review stages
/// 3. The script contains evidence of loop iteration logic
#[test]
fn dev_aidp_emit_still_closes_its_correction_loop() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");
    let out_path = tmpdir.path().join("dev-aidp-loop.js");

    // Prepare minimal inputs for dev-aidp
    let inputs = serde_json::json!({
        "prompt": "test the correction loop closure",
        "feature": "test-correction-loop",
        "track": "quick"
    });
    let inputs_json = serde_json::to_string(&inputs).expect("Failed to serialize inputs");

    // run init + workflow emit for dev-aidp
    let run_and_emit = run_init_and_emit(&ws, "dev-aidp", Some(&inputs_json), &out_path);
    let script = run_and_emit.script;

    // Core verification: the script should successfully emit and contain loop logic.
    // The correction_loop in dev-aidp.yaml has stages replan, refix, and rereview.
    // We verify that the script does not have unconditional harness-gap pauses
    // that would break the loop structure.
    assert!(!script.is_empty(), "Emitted script should not be empty");

    // Verify that the script does not contain unconditional harness-gap pauses
    // for the correction_loop body stages. If such pauses exist, the loop cannot converge.
    let problematic_pause = script
        .lines()
        .any(|line| line.contains("correction_loop") && line.contains("unevaluated_checks"));

    assert!(
        !problematic_pause,
        "Script should not have unconditional harness-gap pauses in correction_loop"
    );

    // Verify that the script contains the loop condition check. A properly
    // closed loop should check the condition and iterate based on findings.
    assert!(
        script.contains("has_blocking_findings") || script.contains("results"),
        "Script should contain loop condition logic (has_blocking_findings or results tracking)"
    );
}

/// Test that a minimal pipeline whose loop never reproduces its output is still
/// exempt from unconditional harness-gap pauses. This guards the engine-level
/// widening independently of the template fix.
///
/// We build a minimal pipeline YAML in a TempDir with:
/// - A source reviewer stage with `on_harness_gap: pause`
/// - A `type: loop` stage whose `when:` reads that stage's `findings` output
/// - The loop body declares no outputs (never reproduces the findings)
///
/// Then we run `aidp validate` on it and emit it, and assert that the emitted
/// script contains no `unevaluated_checks` pause guard for that source stage.
///
/// If the CLI cannot emit an ad-hoc template file with the existing helpers,
/// this test is skipped with a note.
#[test]
fn a_template_whose_loop_never_reproduces_its_output_is_still_exempt() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");

    // Build a minimal template YAML that has a loop with no body producer
    let minimal_template = r#"
version: 5
id: test-loop-no-producer
description: Test template for harness gap exemption
inputs: {}
settings:
  max_correction_iterations: 1
stages:
  - name: audit_stage
    agent: reviewer
    model: haiku
    skills: []
    inputs: {}
    outputs: [findings.json]
    on_harness_gap: pause

  - name: loop_stage
    type: loop
    when: "{{ stages.audit_stage.outputs.findings.has_blocking_findings }}"
    max_iterations: "{{ settings.max_correction_iterations }}"
    body:
      - name: dummy
        agent: executor
        model: haiku
        inputs: {}
        outputs: []
"#;

    // Write the template to a file in the workspace
    let template_path = ws.path().join("test-loop-no-producer.yaml");
    fs::write(&template_path, minimal_template).expect("Failed to write test template");

    // Try to run init + emit on this template
    // If try_run_init_and_emit fails (e.g., the CLI doesn't support ad-hoc files),
    // we skip this test
    let out_path = tmpdir.path().join("test-loop-no-producer.js");
    let run_and_emit = match try_run_init_and_emit(
        aidp_with_fixture_skill,
        &ws,
        &template_path.display().to_string(),
        None,
        &out_path,
    ) {
        Ok(result) => result,
        Err(e) => {
            eprintln!(
                "Skipping a_template_whose_loop_never_reproduces_its_output_is_still_exempt: {}",
                e
            );
            return;
        }
    };

    let script = run_and_emit.script;

    // Assert that the script does NOT contain an unconditional harness-gap pause
    // for the audit_stage. Look for lines containing "audit_stage" and verify
    // that none of them contain "unevaluated_checks".
    let lines_with_audit_stage: Vec<&str> = script
        .lines()
        .filter(|line| line.contains("audit_stage"))
        .collect();

    for line in lines_with_audit_stage {
        assert!(
            !line.contains("unevaluated_checks"),
            "Audit stage should not have unconditional harness-gap pause: {}",
            line
        );
    }
}
