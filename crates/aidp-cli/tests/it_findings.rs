//! Characterization test that freezes the observable contract of `aidp findings merge`.
//!
//! This test file documents the stdout envelope shape (six keys), the golden merged.json file,
//! exit codes (0 for degraded axes, 1 for count mismatches), and the --format quirk where
//! ndjson routes to the same arm as json-pretty.
//!
//! Any change to this test is a deliberate, reviewed break in the binary's interface contract.

use crate::common;
use common::*;
use std::fs;

/// Helper to copy fixture JSON files from the repo's fixtures directory into the workspace.
fn copy_fixtures(ws: &aidp_testkit::TempWorkspace, names: &[&str]) {
    let fixtures_dir = crate_dir().join("../aidp-core/tests/fixtures/findings");
    for name in names {
        let fixture_path = fixtures_dir.join(format!("{}.json", name));
        let dest_filename = format!("{}.json", name);
        let dest_path = ws.child(&dest_filename);
        fs::copy(&fixture_path, &dest_path)
            .unwrap_or_else(|_| panic!("Failed to copy fixture {}.json", name));
    }
}

/// Test 1: Severity counts are recomputed from the findings, not summed from severity_counts fields.
///
/// If we merge correctness (major: 1), perf (major: 0), security (major: 0), and maintainability (minor: 1),
/// the output should have major: 0 (no findings with major severity) and blocking: 2 (two blocking findings
/// from correctness and security). This proves the counts are recalculated.
#[test]
fn merge_recomputes_counts_instead_of_summing() {
    let ws = workspace();
    copy_fixtures(&ws, &["correctness", "perf", "security", "maintainability"]);

    let mut cmd = aidp(&ws);
    cmd.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("correctness.json")
        .arg("--input")
        .arg("perf.json")
        .arg("--input")
        .arg("security.json")
        .arg("--input")
        .arg("maintainability.json")
        .arg("--out")
        .arg("merged.json");

    let output = cmd.output().expect("Failed to run findings merge");
    assert!(
        output.status.success(),
        "findings merge should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stdout_json: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("stdout should be valid JSON");

    // Assert exactly six keys in the envelope
    let keys: Vec<&str> = stdout_json
        .as_object()
        .expect("stdout should be a JSON object")
        .keys()
        .map(|s| s.as_str())
        .collect();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    let mut expected_keys = vec![
        "checks_summary",
        "degraded_axes",
        "has_blocking_findings",
        "merge_log",
        "out",
        "severity_counts",
    ];
    expected_keys.sort();
    assert_eq!(
        sorted_keys, expected_keys,
        "stdout envelope should have exactly these six keys"
    );

    // Read the original correctness.json to get its declared major count
    let correctness_content =
        fs::read_to_string(ws.child("correctness.json")).expect("correctness.json should exist");
    let correctness_json: serde_json::Value =
        serde_json::from_str(&correctness_content).expect("correctness.json should be valid JSON");
    let declared_major = correctness_json["severity_counts"]["major"]
        .as_i64()
        .expect("severity_counts.major should be a number") as i64;

    // Assert the output has major: 0 (not the declared value from any input)
    let output_major = stdout_json["severity_counts"]["major"]
        .as_i64()
        .expect("severity_counts.major should be a number");
    assert_eq!(
        output_major, 0,
        "merged major count should be 0 (recomputed), not the declared {} from correctness",
        declared_major
    );

    // Assert blocking: 2 (one from correctness, one from security)
    let output_blocking = stdout_json["severity_counts"]["blocking"]
        .as_i64()
        .expect("severity_counts.blocking should be a number");
    assert_eq!(output_blocking, 2, "merged blocking count should be 2");

    // Assert minor: 1 (from maintainability)
    let output_minor = stdout_json["severity_counts"]["minor"]
        .as_i64()
        .expect("severity_counts.minor should be a number");
    assert_eq!(output_minor, 1, "merged minor count should be 1");

    // Assert info: 0
    let output_info = stdout_json["severity_counts"]["info"]
        .as_i64()
        .expect("severity_counts.info should be a number");
    assert_eq!(output_info, 0, "merged info count should be 0");

    // Assert degraded_axes is empty (no failures)
    let degraded_axes = stdout_json["degraded_axes"]
        .as_array()
        .expect("degraded_axes should be an array");
    assert!(
        degraded_axes.is_empty(),
        "degraded_axes should be empty for valid inputs"
    );

    // Assert merge_log contains a dedup entry mentioning both source locations
    let merge_log = stdout_json["merge_log"]
        .as_array()
        .expect("merge_log should be an array");
    let has_dedup = merge_log.iter().any(|entry| {
        entry
            .as_str()
            .map(|s| {
                s.starts_with("dedup:")
                    && s.contains("src/hot.rs:100")
                    && s.contains("kept the higher severity (blocking > major)")
            })
            .unwrap_or(false)
    });
    assert!(
        has_dedup,
        "merge_log should contain a dedup entry with severity upgrade info"
    );
}

/// Test 2: checks_summary takes the worst status across all axes.
///
/// When merging correctness (pass/pass), perf (skip/pass), and security (pass/skip),
/// the summary should be {pass:0, fail:0, skip:2} because the worst status is skip.
#[test]
fn checks_summary_takes_the_worst_status_across_axes() {
    let ws = workspace();
    copy_fixtures(&ws, &["correctness", "perf", "security"]);

    let mut cmd = aidp(&ws);
    cmd.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("correctness.json")
        .arg("--input")
        .arg("perf.json")
        .arg("--input")
        .arg("security.json")
        .arg("--out")
        .arg("merged.json");

    let output = cmd.output().expect("Failed to run findings merge");
    assert!(output.status.success());

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stdout_json: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("stdout should be valid JSON");

    let checks_summary = &stdout_json["checks_summary"];
    assert_eq!(checks_summary["pass"], 0, "checks_summary.pass should be 0");
    assert_eq!(checks_summary["fail"], 0, "checks_summary.fail should be 0");
    assert_eq!(checks_summary["skip"], 2, "checks_summary.skip should be 2");
}

/// Test 3: The merged output file is byte-stable except for generated_at.
///
/// This test snapshots the actual merged.json file with insta, redacting only the
/// generated_at field which contains the current timestamp.
#[test]
fn merged_output_file_is_byte_stable_except_generated_at() {
    let ws = workspace();
    copy_fixtures(&ws, &["correctness", "perf", "security"]);

    let mut cmd = aidp(&ws);
    cmd.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("correctness.json")
        .arg("--input")
        .arg("perf.json")
        .arg("--input")
        .arg("security.json")
        .arg("--out")
        .arg("merged.json");

    let output = cmd.output().expect("Failed to run findings merge");
    assert!(output.status.success());

    let merged_path = ws.child("merged.json");
    assert!(
        merged_path.exists(),
        "merged.json should exist after successful merge"
    );

    let merged_content = fs::read_to_string(&merged_path).expect("Should read merged.json");
    let merged_json: serde_json::Value =
        serde_json::from_str(&merged_content).expect("merged.json should be valid JSON");

    // Use insta to snapshot the merged file, redacting only generated_at
    let mut snapshot_value = merged_json.clone();
    snapshot_value["generated_at"] = serde_json::Value::String("[REDACTED]".to_string());

    insta::with_settings!({
        snapshot_suffix => "merged_file",
    }, {
        insta::assert_snapshot!(serde_json::to_string_pretty(&snapshot_value).unwrap());
    });
}

/// Test 4: The stdout envelope is clock-free and byte-identical across runs.
///
/// Run the same merge command twice in the same workspace and assert the stdout
/// is exactly identical both times, proving no clock leakage in the envelope.
#[test]
fn stdout_envelope_is_clock_free_and_byte_identical_across_runs() {
    let ws = workspace();
    copy_fixtures(&ws, &["correctness", "perf", "security"]);

    // First run
    let mut cmd1 = aidp(&ws);
    cmd1.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("correctness.json")
        .arg("--input")
        .arg("perf.json")
        .arg("--input")
        .arg("security.json")
        .arg("--out")
        .arg("merged.json");

    let output1 = cmd1.output().expect("First merge should run");
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout).to_string();

    // Delete the output file between runs to allow the second run to proceed
    let _ = fs::remove_file(ws.child("merged.json"));

    // Second run with the same output file name
    let mut cmd2 = aidp(&ws);
    cmd2.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("correctness.json")
        .arg("--input")
        .arg("perf.json")
        .arg("--input")
        .arg("security.json")
        .arg("--out")
        .arg("merged.json");

    let output2 = cmd2.output().expect("Second merge should run");
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout).to_string();

    // Assert byte-identical
    assert_eq!(
        stdout1, stdout2,
        "stdout envelopes should be byte-identical across runs (no clock)"
    );
}

/// Test 5: Count mismatch hard-fails with exit 1 and writes no output file.
///
/// The maintainability-bad-counts.json fixture declares blocking: 1 but contains only one
/// minor finding. This is the only scenario that causes a hard failure (exit 1).
#[test]
fn count_mismatch_hard_fails_and_writes_no_output() {
    let ws = workspace();
    copy_fixtures(&ws, &["maintainability-bad-counts"]);

    let mut cmd = aidp(&ws);
    cmd.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("maintainability-bad-counts.json")
        .arg("--out")
        .arg("merged.json");

    let output = cmd.output().expect("Failed to run findings merge");
    assert_eq!(
        output.status.code(),
        Some(1),
        "findings merge should exit 1 for count mismatch"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("maintainability-bad-counts"),
        "stderr should mention the problematic file"
    );
    assert!(
        stderr.contains("does not match the 1 finding(s) actually listed"),
        "stderr should mention the count mismatch"
    );
    assert!(
        stderr.contains("misreported its own count"),
        "stderr should mention the misreport"
    );

    // FIGE-A-DESSEIN: the error is wrapped twice by merge and load; LOC-39 owns cleanup
    let count = stderr.matches("maintainability-bad-counts.json").count();
    assert_eq!(
        count, 2,
        "file name should appear exactly twice in stderr (wrapped by merge and load)"
    );

    // Assert the output file was NOT created
    assert!(
        !ws.child("merged.json").exists(),
        "merged.json should not exist after hard failure"
    );
}

/// Test 6: Legacy axis without checks field is degraded, not fatal.
///
/// This is a non-regression test for the 'review synthesis blocked by findings schema' incident.
/// The legacy-no-checks.json fixture is missing the required `checks` field. This should
/// degrade the axis (exit 0) rather than fail the entire merge.
#[test]
fn legacy_axis_without_checks_is_degraded_not_fatal() {
    let ws = workspace();
    copy_fixtures(&ws, &["correctness", "legacy-no-checks"]);

    let mut cmd = aidp(&ws);
    cmd.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("correctness.json")
        .arg("--input")
        .arg("legacy-no-checks.json")
        .arg("--out")
        .arg("merged.json");

    let output = cmd.output().expect("Failed to run findings merge");
    assert!(
        output.status.success(),
        "findings merge should exit 0 (NOT 1) for schema violation after LOC-42"
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stdout_json: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("stdout should be valid JSON");

    let degraded_axes = stdout_json["degraded_axes"]
        .as_array()
        .expect("degraded_axes should be an array");
    assert!(
        degraded_axes
            .iter()
            .any(|v| v.as_str() == Some("legacy-no-checks")),
        "legacy-no-checks should be in degraded_axes"
    );

    let merge_log = stdout_json["merge_log"]
        .as_array()
        .expect("merge_log should be an array");

    // Use separate contains assertions (jsonschema crate does not specify violation order)
    let log_text = merge_log
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        log_text.contains("findings violate schema/findings.schema.json"),
        "merge_log should mention schema violation"
    );
    assert!(
        log_text.contains("\"checks\" is a required property"),
        "merge_log should mention the missing checks field"
    );
    assert!(
        log_text.contains("legacy-no-checks"),
        "merge_log should mention the axis label"
    );

    // Assert the output file exists and still contains correctness's finding
    assert!(
        ws.child("merged.json").exists(),
        "merged.json should exist even with degraded axis"
    );
    let merged_content =
        fs::read_to_string(ws.child("merged.json")).expect("Should read merged.json");
    let merged_json: serde_json::Value =
        serde_json::from_str(&merged_content).expect("merged.json should be valid JSON");
    let findings = merged_json["findings"]
        .as_array()
        .expect("findings should be an array");
    assert!(
        !findings.is_empty(),
        "merged.json should contain correctness's findings despite degraded legacy-no-checks"
    );
}

/// Test 7: Invalid JSON input is degraded with a distinct error message.
///
/// Write a malformed JSON file and assert it is degraded (exit 0), with a message
/// that mentions "not valid JSON" (distinct from the schema violation message).
#[test]
fn invalid_json_degrades_with_a_message_distinct_from_the_schema_one() {
    let ws = workspace();
    let broken_path = ws.child("broken.json");
    fs::write(&broken_path, "{ not json").expect("Should write broken.json");

    let mut cmd = aidp(&ws);
    cmd.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("broken.json")
        .arg("--out")
        .arg("merged.json");

    let output = cmd.output().expect("Failed to run findings merge");
    assert!(
        output.status.success(),
        "findings merge should exit 0 for invalid JSON (degraded, not fatal)"
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stdout_json: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("stdout should be valid JSON");

    let degraded_axes = stdout_json["degraded_axes"]
        .as_array()
        .expect("degraded_axes should be an array");
    assert!(
        degraded_axes.iter().any(|v| v.as_str() == Some("broken")),
        "broken should be in degraded_axes"
    );

    let merge_log = stdout_json["merge_log"]
        .as_array()
        .expect("merge_log should be an array");
    let log_text = merge_log
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        log_text.contains("findings is not valid JSON"),
        "merge_log should mention JSON parsing failure"
    );
    assert!(
        !log_text.contains("findings violate schema/findings.schema.json"),
        "merge_log should NOT mention schema violation (distinct from JSON error)"
    );
}

/// Test 8: Missing input file is named and degraded.
///
/// Do not create the input file and assert it is degraded (exit 0) with a message
/// naming the file and the read failure, without asserting the OS error text.
#[test]
fn missing_input_file_is_named_and_degraded() {
    let ws = workspace();

    let mut cmd = aidp(&ws);
    cmd.arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("nope.json")
        .arg("--out")
        .arg("merged.json");

    let output = cmd.output().expect("Failed to run findings merge");
    assert!(
        output.status.success(),
        "findings merge should exit 0 for missing file (degraded, not fatal)"
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stdout_json: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("stdout should be valid JSON");

    let degraded_axes = stdout_json["degraded_axes"]
        .as_array()
        .expect("degraded_axes should be an array");
    assert!(
        degraded_axes.iter().any(|v| v.as_str() == Some("nope")),
        "nope should be in degraded_axes"
    );

    let merge_log = stdout_json["merge_log"]
        .as_array()
        .expect("merge_log should be an array");
    let log_text = merge_log
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        log_text.contains("findings entry"),
        "merge_log should mention findings entry"
    );
    assert!(
        log_text.contains("nope"),
        "merge_log should name the missing file"
    );
    assert!(
        log_text.contains("read findings"),
        "merge_log should mention read failure"
    );
    // Note: we don't assert the OS error text itself (differs between Windows/Linux)
}

/// Test 9: both a missing axis and a missing `--out` are usage errors.
///
/// Two invocations: (a) with --out but no inputs → exit 7, a `TypedExit` carrying
/// `ExitStatus::Usage`. (b) inputs but no --out → exit 7, clap's own usage message.
#[test]
fn no_axis_and_missing_out_both_exit_as_usage_errors() {
    let ws = workspace();

    // Invocation (a): with --out but no --input
    let mut cmd_a = aidp(&ws);
    cmd_a
        .arg("findings")
        .arg("merge")
        .arg("--out")
        .arg("merged.json");

    let output_a = cmd_a.output().expect("Failed to run findings merge");
    assert_eq!(
        output_a.status.code(),
        Some(7),
        "findings merge with no inputs but with --out should exit 7 (usage)"
    );

    let stderr_a = String::from_utf8_lossy(&output_a.stderr);
    assert_eq!(
        stderr_a.trim(),
        "Error: findings merge needs at least one --input or --label axis=path",
        "stderr should be exactly the TypedExit message"
    );

    // Invocation (b): with input but no --out
    copy_fixtures(&ws, &["correctness"]);
    let mut cmd_b = aidp(&ws);
    cmd_b
        .arg("findings")
        .arg("merge")
        .arg("--input")
        .arg("correctness.json");

    let output_b = cmd_b.output().expect("Failed to run findings merge");
    assert_eq!(
        output_b.status.code(),
        Some(7),
        "findings merge without --out should exit 7 (clap usage error)"
    );

    let stderr_b = String::from_utf8_lossy(&output_b.stderr);
    assert!(
        stderr_b.contains("Usage:"),
        "stderr should contain a Usage: line (clap error)"
    );
    assert!(
        stderr_b.contains("findings merge --out"),
        "stderr should mention --out in the usage line"
    );
}

/// `--format ndjson` prints one compact JSON value per line, equal to the compact default.
/// `--format json-pretty` spans several lines.
#[test]
fn format_ndjson_is_line_delimited_json() {
    let ws = workspace();
    copy_fixtures(&ws, &["security"]);

    // Run with --format ndjson
    let mut cmd_ndjson = aidp(&ws);
    cmd_ndjson
        .arg("findings")
        .arg("merge")
        .arg("--label")
        .arg("security=security.json")
        .arg("--out")
        .arg("merged.json")
        .arg("--format")
        .arg("ndjson");

    let output_ndjson = cmd_ndjson.output().expect("Failed to run with ndjson");
    assert!(output_ndjson.status.success());
    let stdout_ndjson = String::from_utf8_lossy(&output_ndjson.stdout).to_string();

    // Delete the output file between runs
    let _ = fs::remove_file(ws.child("merged.json"));

    // Run with --format json-pretty
    let mut cmd_pretty = aidp(&ws);
    cmd_pretty
        .arg("findings")
        .arg("merge")
        .arg("--label")
        .arg("security=security.json")
        .arg("--out")
        .arg("merged.json")
        .arg("--format")
        .arg("json-pretty");

    let output_pretty = cmd_pretty.output().expect("Failed to run with json-pretty");
    assert!(output_pretty.status.success());
    let stdout_pretty = String::from_utf8_lossy(&output_pretty.stdout).to_string();

    // Delete the output file between runs
    let _ = fs::remove_file(ws.child("merged.json"));

    // Run with no --format (compact)
    let mut cmd_compact = aidp(&ws);
    cmd_compact
        .arg("findings")
        .arg("merge")
        .arg("--label")
        .arg("security=security.json")
        .arg("--out")
        .arg("merged.json");

    let output_compact = cmd_compact.output().expect("Failed to run with no format");
    assert!(output_compact.status.success());
    let stdout_compact = String::from_utf8_lossy(&output_compact.stdout).to_string();

    assert_eq!(
        stdout_ndjson.trim(),
        stdout_compact.trim(),
        "ndjson and compact should produce identical output"
    );
    assert_eq!(stdout_ndjson.trim().lines().count(), 1);
    serde_json::from_str::<serde_json::Value>(stdout_ndjson.trim())
        .expect("ndjson line should parse as JSON");

    assert!(
        stdout_pretty.trim().contains('\n'),
        "json-pretty output should span multiple lines"
    );
}
