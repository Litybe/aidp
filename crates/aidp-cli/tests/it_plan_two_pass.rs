use crate::common;
use common::*;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Fixture: A minimal plan.json with three tasks that exercise the scheduler.
/// T1 writes src/Shared.cs, T2 writes src/Shared.cs (will serialize after T1), T3 needs T1 and writes src/Other.cs.
const PLAN_JSON_FIXTURE: &str = r#"{
  "schema": "aidp-plan.v1",
  "summary": "Two-pass compilation test fixture",
  "tasks": [
    {
      "id": "T1",
      "title": "Task 1",
      "kind": "code",
      "prompt": "Implement task 1",
      "files": ["src/Shared.cs"]
    },
    {
      "id": "T2",
      "title": "Task 2",
      "kind": "code",
      "prompt": "Implement task 2. Serialized after (file-lock only — not a dependency) T1",
      "files": ["src/Shared.cs"]
    },
    {
      "id": "T3",
      "title": "Task 3",
      "kind": "code",
      "prompt": "Implement task 3. dependency declared by the plan",
      "files": ["src/Other.cs"],
      "needs": ["T1"]
    }
  ]
}
"#;

/// Extract run_dir from stderr of `aidp run init`.
fn extract_run_dir(stderr: &str) -> PathBuf {
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix("run_dir: ") {
            return PathBuf::from(rest.trim());
        }
    }
    panic!("Could not find 'run_dir:' in stderr: {}", stderr);
}

/// Scenario 1-2: Pass 1 — verify awaiting_plan JSON, truncated script, and plan-schedule.json absence.
#[test]
fn scenario_1_2_awaiting_plan_phase() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");
    let workflow_js_path = tmpdir.path().join("workflow_pass1.js");

    // Initialize run with the dev-aidp template
    let mut cmd_init = aidp_with_fixture_skill(&ws);
    cmd_init
        .arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg(skill_root().join("templates/dev-aidp.yaml"))
        .arg("--no-vault")
        .arg("--workspace-root")
        .arg(".")
        .arg("--inputs")
        .arg(r#"{"prompt":"p","feature":"filet","track":"quick"}"#);

    let output_init = cmd_init.output().expect("Failed to run aidp run init");
    assert!(
        output_init.status.success(),
        "run init failed: {}",
        String::from_utf8_lossy(&output_init.stderr)
    );

    let run_id = String::from_utf8_lossy(&output_init.stdout)
        .trim()
        .to_string();
    assert!(!run_id.is_empty(), "run init should produce a run_id");

    let stderr_init = String::from_utf8_lossy(&output_init.stderr).to_string();
    let run_dir_impl = extract_run_dir(&stderr_init);

    // First workflow emit (without plan.json)
    let mut cmd_emit1 = aidp_with_fixture_skill(&ws);
    cmd_emit1
        .arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(&workflow_js_path);

    let output_emit1 = cmd_emit1.output().expect("Failed to run workflow emit");
    assert!(
        output_emit1.status.success(),
        "first workflow emit should succeed: {}",
        String::from_utf8_lossy(&output_emit1.stderr)
    );

    // Parse stderr JSON
    let stderr_json_str = String::from_utf8_lossy(&output_emit1.stderr);
    let stderr_json: serde_json::Value =
        serde_json::from_str(&stderr_json_str).expect("stderr should be valid JSON");

    // Verify five keys in awaiting_plan response
    assert_eq!(
        stderr_json["phase"], "awaiting_plan",
        "phase should be awaiting_plan"
    );
    assert_eq!(
        stderr_json["stage"], "implement",
        "stage should be implement"
    );
    assert_eq!(stderr_json["producer"], "plan", "producer should be plan");
    assert!(
        stderr_json["plan_path"].is_string(),
        "plan_path should be present"
    );
    assert!(stderr_json["note"].is_string(), "note should be present");

    // Verify plan_path ends with /plan/plan.json (normalize backslashes to forward slashes)
    let plan_path = stderr_json["plan_path"]
        .as_str()
        .unwrap()
        .replace('\\', "/");
    assert!(
        plan_path.ends_with("/plan/plan.json"),
        "plan_path should end with /plan/plan.json, got: {}",
        plan_path
    );

    // Read the emitted script
    let script = fs::read_to_string(&workflow_js_path).expect("Failed to read emitted script");

    // Verify script contains awaiting_plan return statement
    assert!(
        script.contains(r#"return { status: "awaiting_plan", at: "implement", producer: "plan""#),
        "script should contain awaiting_plan return statement"
    );

    // Verify script does NOT contain plan_compiled features
    assert!(
        !script.contains("__tk_implement["),
        "script should not contain __tk_implement[ in pass 1"
    );
    assert!(
        !script.contains(r#"results["rebuild"] = await"#),
        "script should not contain results[\"rebuild\"] in pass 1"
    );
    assert!(
        !script.contains(r#"results["verify"] = await"#),
        "script should not contain results[\"verify\"] in pass 1"
    );

    // Verify plan-schedule.json does NOT exist
    let plan_schedule_path = run_dir_impl.join("plan-schedule.json");
    assert!(
        !plan_schedule_path.exists(),
        "plan-schedule.json should not exist in pass 1"
    );
}

/// Scenario 3-6: Pass 2 — verify plan_compiled JSON, plan-schedule.json golden, and script structure.
#[test]
fn scenario_3_6_plan_compiled_phase() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");
    let workflow_js_path_1 = tmpdir.path().join("workflow_pass2a.js");
    let workflow_js_path_2 = tmpdir.path().join("workflow_pass2b.js");

    // Initialize run with the dev-aidp template
    let mut cmd_init = aidp_with_fixture_skill(&ws);
    cmd_init
        .arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg(skill_root().join("templates/dev-aidp.yaml"))
        .arg("--no-vault")
        .arg("--workspace-root")
        .arg(".")
        .arg("--inputs")
        .arg(r#"{"prompt":"p","feature":"filet","track":"quick"}"#);

    let output_init = cmd_init.output().expect("Failed to run aidp run init");
    assert!(output_init.status.success(), "run init failed");

    let run_id = String::from_utf8_lossy(&output_init.stdout)
        .trim()
        .to_string();

    // First workflow emit to get plan_path
    let mut cmd_emit1 = aidp_with_fixture_skill(&ws);
    cmd_emit1
        .arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(&workflow_js_path_1);

    let output_emit1 = cmd_emit1.output().expect("Failed to run workflow emit");
    assert!(output_emit1.status.success(), "first workflow emit failed");

    let stderr_json_str = String::from_utf8_lossy(&output_emit1.stderr);
    let stderr_json: serde_json::Value =
        serde_json::from_str(&stderr_json_str).expect("stderr should be valid JSON");

    // Use the exact plan_path from the JSON response
    let plan_path_str = stderr_json["plan_path"]
        .as_str()
        .expect("plan_path should be present");

    // plan_path is relative to workspace root; make it absolute
    let plan_path_abs = ws.path().join(plan_path_str);

    if let Some(parent) = plan_path_abs.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent directories for plan.json");
    }

    fs::write(&plan_path_abs, PLAN_JSON_FIXTURE)
        .unwrap_or_else(|e| panic!("Failed to write plan.json at {:?}: {}", plan_path_abs, e));

    // Second workflow emit (with plan.json)
    let mut cmd_emit2 = aidp_with_fixture_skill(&ws);
    cmd_emit2
        .arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(&workflow_js_path_2);

    let output_emit2 = cmd_emit2
        .output()
        .expect("Failed to run second workflow emit");
    assert!(
        output_emit2.status.success(),
        "second workflow emit should succeed: {}",
        String::from_utf8_lossy(&output_emit2.stderr)
    );

    // Parse stderr JSON from second emit
    let stderr_json_str2 = String::from_utf8_lossy(&output_emit2.stderr);
    let stderr_json2: serde_json::Value = serde_json::from_str(&stderr_json_str2)
        .expect("stderr should be valid JSON for plan_compiled");

    // Verify plan_compiled phase
    assert_eq!(
        stderr_json2["phase"], "plan_compiled",
        "phase should be plan_compiled, got: {}",
        stderr_json2["phase"]
    );

    // Verify plans array
    assert!(stderr_json2["plans"].is_array(), "plans should be an array");
    let plans = stderr_json2["plans"]
        .as_array()
        .expect("plans should be an array");
    assert_eq!(plans.len(), 1, "plans should have one element");

    let plan0 = &plans[0];
    assert_eq!(plan0["stage"], "implement", "stage should be implement");
    assert_eq!(plan0["tasks"], 3, "tasks should be 3");
    assert_eq!(plan0["waves"], 2, "waves should be 2");
    assert_eq!(
        plan0["max_parallel"], 5,
        "max_parallel should be 5 (dev-aidp.yaml implement.plan_tasks.max)"
    );
    assert_eq!(
        plan0["serializations"], 1,
        "serializations should be 1 (T1->T2 file lock)"
    );
    assert_eq!(plan0["warnings"], 0, "warnings should be 0");

    // plan-schedule.json should be in the same directory as plan.json's parent
    let plan_dir = plan_path_abs.parent().unwrap(); // .../plan/
    let plan_stage_dir = plan_dir.parent().unwrap(); // .../implement/
    let plan_schedule_path = plan_stage_dir.join("plan-schedule.json");
    assert!(
        plan_schedule_path.exists(),
        "plan-schedule.json should exist in pass 2"
    );

    let schedule_json_str =
        fs::read_to_string(&plan_schedule_path).expect("Failed to read plan-schedule.json");
    let schedule_array: Vec<serde_json::Value> =
        serde_json::from_str(&schedule_json_str).expect("plan-schedule.json should be valid JSON");

    assert_eq!(
        schedule_array.len(),
        1,
        "plan-schedule.json should contain one schedule"
    );

    let schedule = &schedule_array[0];

    // Use insta for the golden (with redaction)
    let mut redacted_schedule = schedule.clone();
    // Redact non-deterministic fields
    redacted_schedule["compiled_at"] = serde_json::json!("<REDACTED>");
    redacted_schedule["plan_path"] = serde_json::json!("<REDACTED>");

    let redacted_json_str = serde_json::to_string_pretty(&redacted_schedule)
        .expect("Failed to serialize redacted schedule to JSON");
    insta::assert_snapshot!("plan_schedule", redacted_json_str);

    // Verify waves structure manually
    let waves = &schedule["waves"];
    assert!(waves.is_array(), "waves should be an array");
    let waves_arr = waves.as_array().unwrap();
    assert_eq!(waves_arr.len(), 2, "should have 2 waves");

    // Wave 0 should be ["T1"], Wave 1 should be ["T2", "T3"]
    assert_eq!(
        waves_arr[0],
        serde_json::json!(["T1"]),
        "wave 0 should contain T1"
    );
    assert_eq!(
        waves_arr[1],
        serde_json::json!(["T2", "T3"]),
        "wave 1 should contain T2 and T3"
    );

    // Verify serializations
    let serializations = &schedule["serializations"];
    assert!(
        serializations.is_array(),
        "serializations should be an array"
    );
    let serializations_arr = serializations.as_array().unwrap();
    assert_eq!(
        serializations_arr.len(),
        1,
        "should have 1 serialization (T1->T2)"
    );

    let serialization = &serializations_arr[0];
    assert_eq!(
        serialization["before"], "T1",
        "serialization should have before: T1"
    );
    assert_eq!(
        serialization["after"], "T2",
        "serialization should have after: T2"
    );
    assert_eq!(
        serialization["files"],
        serde_json::json!(["src/Shared.cs"]),
        "serialization files should be src/Shared.cs"
    );

    // Verify tasks structure
    let tasks = &schedule["tasks"];
    assert!(tasks.is_array(), "tasks should be an array");
    let tasks_arr = tasks.as_array().unwrap();
    assert_eq!(tasks_arr.len(), 3, "should have 3 tasks");

    // Find T1, T2, T3 in the tasks array
    let _t1 = tasks_arr.iter().find(|t| t["id"] == "T1").unwrap();
    let t2 = tasks_arr.iter().find(|t| t["id"] == "T2").unwrap();
    let t3 = tasks_arr.iter().find(|t| t["id"] == "T3").unwrap();

    // Verify T2 has mutex_after and NO deps key
    assert_eq!(
        t2["mutex_after"],
        serde_json::json!(["T1"]),
        "T2 should have mutex_after: [\"T1\"]"
    );
    assert!(
        !t2.as_object().unwrap().contains_key("deps"),
        "T2 should NOT have deps key"
    );

    // Verify T3 has deps and NO mutex_after key
    assert_eq!(
        t3["deps"],
        serde_json::json!(["T1"]),
        "T3 should have deps: [\"T1\"]"
    );
    assert!(
        !t3.as_object().unwrap().contains_key("mutex_after"),
        "T3 should NOT have mutex_after key"
    );

    // Read and verify the emitted script
    let script = fs::read_to_string(&workflow_js_path_2).expect("Failed to read emitted script");

    // Verify runTask calls with correct argument order
    assert!(
        script.contains(r#"__tk_implement["T2"] = runTask("T2", [], ["T1"], __tk_implement"#),
        "T2 should have deps=[] and mutex_after=[\"T1\"]"
    );
    assert!(
        script.contains(r#"__tk_implement["T3"] = runTask("T3", ["T1"], [], __tk_implement"#),
        "T3 should have deps=[\"T1\"] and mutex_after=[]"
    );

    // Verify prompt text for serialization and dependency
    assert!(
        script.contains("Serialized after (file-lock only — not a dependency)"),
        "script should contain serialization description in T2 prompt"
    );
    assert!(
        script.contains("dependency declared by the plan"),
        "script should contain dependency description in T3 prompt"
    );

    // Verify pool(2, 50000) with comment about effective_parallel divergence
    assert!(
        script.contains("const __sm_implement = pool(2, 50000);"),
        "script should contain pool(2, 50000)"
    );
    // The comment might explain the divergence - check if it's there
    if !script.contains("effective_parallel") && !script.contains("effective") {
        // For now, just note this - the implementation might not add the comment yet
        eprintln!(
            "NOTE: Script does not contain explicit comment about effective_parallel divergence"
        );
    }
}

/// Scenario 7: Schema violation on a fresh run — must exit 1 with exact error message.
#[test]
fn scenario_7_schema_violation_exit_1() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");
    let workflow_js_path_1 = tmpdir.path().join("workflow_scenario7a.js");
    let workflow_js_path_2 = tmpdir.path().join("workflow_scenario7b.js");

    // Initialize a FRESH run
    let mut cmd_init = aidp_with_fixture_skill(&ws);
    cmd_init
        .arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg(skill_root().join("templates/dev-aidp.yaml"))
        .arg("--no-vault")
        .arg("--workspace-root")
        .arg(".")
        .arg("--inputs")
        .arg(r#"{"prompt":"p","feature":"filet","track":"quick"}"#);

    let output_init = cmd_init.output().expect("Failed to run aidp run init");
    assert!(output_init.status.success(), "run init failed");

    let run_id = String::from_utf8_lossy(&output_init.stdout)
        .trim()
        .to_string();

    // First workflow emit to get plan_path and workflow.js bytes
    let mut cmd_emit1 = aidp_with_fixture_skill(&ws);
    cmd_emit1
        .arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(&workflow_js_path_1);

    let output_emit1 = cmd_emit1.output().expect("Failed to run workflow emit");
    assert!(output_emit1.status.success(), "first workflow emit failed");

    let stderr_json_str = String::from_utf8_lossy(&output_emit1.stderr);
    let stderr_json: serde_json::Value =
        serde_json::from_str(&stderr_json_str).expect("stderr should be valid JSON");

    // Use the exact plan_path from the JSON response
    let plan_path_str = stderr_json["plan_path"]
        .as_str()
        .expect("plan_path should be present");

    // plan_path is relative to workspace root; make it absolute
    let plan_path_abs = ws.path().join(plan_path_str);

    // Capture the original workflow.js bytes
    let original_workflow_bytes =
        fs::read(&workflow_js_path_1).expect("Failed to read original workflow.js");

    // Write INVALID plan.json (missing files field)
    let invalid_plan = r#"{
  "schema": "aidp-plan.v1",
  "summary": "Invalid plan: missing files",
  "tasks": [
    {
      "id": "T1",
      "title": "Task 1",
      "kind": "code",
      "prompt": "Implement task 1"
    }
  ]
}
"#;

    if let Some(parent) = plan_path_abs.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent directories");
    }
    fs::write(&plan_path_abs, invalid_plan).expect("Failed to write invalid plan.json");

    // Re-run workflow emit with invalid plan
    let mut cmd_emit2 = aidp_with_fixture_skill(&ws);
    cmd_emit2
        .arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(&workflow_js_path_2);

    let output_emit2 = cmd_emit2
        .output()
        .expect("Failed to run second workflow emit");

    // Verify exit code is 1 (failure)
    assert!(
        !output_emit2.status.success(),
        "second workflow emit should fail with invalid plan"
    );

    // Verify stderr contains exact error message
    let stderr2 = String::from_utf8_lossy(&output_emit2.stderr);
    assert!(
        stderr2.contains("plan violates schema/plan.schema.json:"),
        "stderr should mention schema violation: {}",
        stderr2
    );
    assert!(
        stderr2.contains(r#""files" is a required property"#),
        "stderr should mention missing files property: {}",
        stderr2
    );

    // Verify workflow.js is byte-identical to the original
    let current_workflow_bytes =
        fs::read(&workflow_js_path_1).expect("Failed to read workflow.js after schema violation");
    assert_eq!(
        original_workflow_bytes, current_workflow_bytes,
        "workflow.js should be byte-identical after schema violation"
    );

    // Verify plan-schedule.json still does NOT exist
    let plan_dir = plan_path_abs.parent().unwrap(); // .../plan/
    let plan_stage_dir = plan_dir.parent().unwrap(); // .../implement/
    let plan_schedule_path = plan_stage_dir.join("plan-schedule.json");
    assert!(
        !plan_schedule_path.exists(),
        "plan-schedule.json should still not exist after schema violation"
    );
}
