use crate::common;
#[path = "support/prompt_examples.rs"]
mod prompt_examples;

use common::*;
use std::fs;
use tempfile::TempDir;

const MAX_WORKERS: usize = 4;

/// Snapshot redaction: replace the run id, then rewrite every emitted
/// workspace path (`./.aidp/...`) to forward slashes.
///
/// Only a path run is touched, so the JS escapes elsewhere in the script
/// (`\n`, `\s`, `\u200B`, ...) stay verbatim on every OS. Inside a JS
/// string literal a Windows separator is the two-character `\\`, while a
/// newline escape is the one-character-backslash `\n`, so the two never
/// collide: a run starts at `.` + separator + `.aidp` and a separator is
/// only recognised with the same width as the one that opened the run.
fn redact_script(script: &str, run_id: &str) -> String {
    normalize_workspace_paths(&script.replace(run_id, "<RUN_ID>"))
}

fn normalize_workspace_paths(s: &str) -> String {
    fn is_segment(c: u8) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'<' | b'>')
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        let width = if b[i] != b'.' {
            None
        } else if b[i + 1..].starts_with(b"/.aidp") {
            // Already a forward-slash path: no backslash is a separator here.
            Some(0)
        } else if b[i + 1..].starts_with(b"\\\\.aidp") {
            Some(2)
        } else if b[i + 1..].starts_with(b"\\.aidp") {
            Some(1)
        } else {
            None
        };
        let Some(width) = width else {
            i += 1;
            continue;
        };
        out.push_str(&s[copied..i]);
        out.push_str("./.aidp");
        i += 1 + width.max(1) + ".aidp".len();
        loop {
            if i < b.len() && (is_segment(b[i]) || b[i] == b'/') {
                out.push(b[i] as char);
                i += 1;
            } else if width > 0
                && b[i..].iter().take(width).all(|&c| c == b'\\')
                && b.len() > i + width
                && is_segment(b[i + width])
            {
                out.push('/');
                i += width;
            } else {
                break;
            }
        }
        copied = i;
    }
    out.push_str(&s[copied..]);
    out
}

/// The golden snapshots are recorded on one OS and checked on both, so the
/// redaction must map both renderings of the same script to one string.
#[test]
fn normalize_workspace_paths_is_os_independent() {
    let linux = r#""- `in`: ./.aidp/pipelines/p/scan_data/data.md\n- `x`: \\s\n""#;
    let windows = r#""- `in`: .\\.aidp\\pipelines\\p\\scan_data\\data.md\n- `x`: \\s\n""#;
    assert_eq!(normalize_workspace_paths(linux), linux);
    assert_eq!(normalize_workspace_paths(windows), linux);
}

/// Test that two successive `workflow emit` passes on the same run produce
/// byte-identical files. This verifies determinism in the compilation pipeline.
#[test]
fn emit_is_byte_identical_across_two_passes() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");
    let pass1_path = tmpdir.path().join("pass1.js");

    // First part: run init + first emit
    let run_and_emit = run_init_and_emit(&ws, "study", Some(r#"{"topic":"t"}"#), &pass1_path);
    let run_id = run_and_emit.run_id;

    // Verify first emit stdout
    assert!(
        !run_and_emit.script.is_empty(),
        "first emit should produce a script"
    );

    // Second emit pass (keep separate to verify stdout)
    let pass2_path = tmpdir.path().join("pass2.js");
    let mut cmd2 = aidp_with_fixture_skill(&ws);
    cmd2.arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(&pass2_path);

    let output2 = cmd2.output().expect("Failed to run second workflow emit");
    assert!(
        output2.status.success(),
        "second workflow emit should succeed: {}",
        String::from_utf8_lossy(&output2.stderr)
    );

    let stdout2 = String::from_utf8_lossy(&output2.stdout).trim().to_string();
    assert_eq!(
        stdout2,
        pass2_path.display().to_string(),
        "second emit stdout should be the exact out path"
    );

    // Compare as Vec<u8> for byte-identity
    let bytes1 = fs::read(&pass1_path).expect("Failed to read pass1.js");
    let bytes2 = fs::read(&pass2_path).expect("Failed to read pass2.js");
    assert_eq!(
        bytes1, bytes2,
        "Two emit passes should produce byte-identical files"
    );
}

/// Test that `run init` without a required input is rejected.
#[test]
fn run_init_rejects_a_missing_required_input() {
    let ws = workspace();
    let mut cmd = aidp_with_fixture_skill(&ws);

    cmd.arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg("study")
        .arg("--no-vault")
        .arg("--workspace-root")
        .arg(".");
    // Note: no --inputs provided, and 'topic' is required

    let output = cmd.output().expect("Failed to run aidp run init");
    assert!(
        !output.status.success(),
        "run init without required input should fail"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("required input 'topic' not provided"),
        "stderr should mention missing required input: {}",
        stderr
    );

    // Check that the run directory was NOT created
    let no_ticket_dir = ws.path().join(".aidp/pipelines/_no-ticket");
    assert!(
        !no_ticket_dir.exists(),
        "run directory should not be created on input validation failure"
    );
}

/// Test that `validate --all --workspace-root .` accepts every shipped template,
/// returns ok: true, and reports the correct count.
#[test]
fn validate_all_accepts_every_shipped_template() {
    let ws = workspace();
    let mut cmd = aidp_with_fixture_skill(&ws);

    cmd.arg("validate")
        .arg("--all")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run validate --all");
    assert!(output.status.success(), "validate --all should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value =
        serde_json::from_str(&stdout).expect("validate --all stdout should be valid JSON");

    assert_eq!(result["ok"], true, "validate --all should report ok: true");
    assert_eq!(
        result["failed"], 0,
        "validate --all should report failed: 0"
    );

    let total = result["total"].as_u64().expect("total should be a number");
    let ids: Vec<String> = result["results"]
        .as_array()
        .expect("results should be an array")
        .iter()
        .map(|r| r["id"].as_str().unwrap_or("").to_string())
        .collect();

    // Verify count matches template_ids()
    let expected_ids = template_ids();
    assert_eq!(
        total as usize,
        expected_ids.len(),
        "total should match the number of template files (expected at least 39)"
    );
    assert!(
        total >= 39,
        "validate --all should find at least 39 templates; measured: {}",
        total
    );

    // Verify that dev, study, audit, dev-composed are present
    assert!(
        ids.contains(&"dev".to_string()),
        "results should contain 'dev'"
    );
    assert!(
        ids.contains(&"study".to_string()),
        "results should contain 'study'"
    );
    assert!(
        ids.contains(&"audit".to_string()),
        "results should contain 'audit'"
    );
    assert!(
        ids.contains(&"dev-composed".to_string()),
        "results should contain 'dev-composed'"
    );
}

/// Number of workers: available cores, capped at `MAX_WORKERS` and the item count.
fn worker_count_for(item_count: usize) -> usize {
    std::thread::available_parallelism()
        .map(|cores| cores.get())
        .unwrap_or(1)
        .min(MAX_WORKERS)
        .min(item_count)
        .max(1)
}

/// Runs init, emit and `node --check` for one template in its own workspace.
fn check_template(template_id: &str) -> Result<(), String> {
    let workspace_dir = workspace();
    let tmpdir = TempDir::new().map_err(|err| format!("{template_id}: tempdir: {err}"))?;
    let inputs_json = serde_json::to_string(&inputs_for_template(template_id))
        .map_err(|err| format!("{template_id}: serialize inputs: {err}"))?;
    let out_path = tmpdir.path().join(format!("{template_id}.js"));
    let emitted = try_run_init_and_emit(
        aidp_with_fixture_skill,
        &workspace_dir,
        template_id,
        Some(&inputs_json),
        &out_path,
    )
    .map_err(|err| format!("{template_id}: {err}"))?;

    let check_file = tmpdir.path().join(format!("{template_id}.check.js"));
    fs::write(&check_file, wrap_for_node_check(&emitted.script))
        .map_err(|err| format!("{template_id}: write wrapped script: {err}"))?;
    let output = std::process::Command::new("node")
        .arg("--check")
        .arg(&check_file)
        .output()
        .map_err(|err| format!("{template_id}: run node --check: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{template_id}: node --check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn check_templates_chunk(chunk: &[String]) -> Vec<Result<(), String>> {
    chunk.iter().map(|id| check_template(id)).collect()
}

/// Test that emitted JavaScript parses for every shipped template.
/// Requires `node` to be available; skips if not.
#[test]
fn emitted_javascript_parses_for_every_template() {
    if !node_available() {
        eprintln!(
            "node executable not found; skipping emitted_javascript_parses_for_every_template"
        );
        return;
    }

    let template_list = template_ids();
    let worker_count = worker_count_for(template_list.len());
    let chunk_size = template_list.len().div_ceil(worker_count).max(1);

    let outcomes: Vec<Result<(), String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = template_list
            .chunks(chunk_size)
            .map(|chunk| scope.spawn(move || check_templates_chunk(chunk)))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("template worker panicked"))
            .collect()
    });

    let processed_count = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    let failed: Vec<String> = outcomes.into_iter().filter_map(Result::err).collect();

    assert!(
        failed.is_empty(),
        "All templates should initialize and emit successfully. Failed templates:\n{}",
        failed.join("\n")
    );
    assert_eq!(
        processed_count,
        template_list.len(),
        "Should have processed all {} templates",
        template_list.len()
    );
    assert!(
        processed_count >= 39,
        "Should have processed at least 39 templates; processed: {}",
        processed_count
    );
}

/// Test that raw emitted JavaScript (without wrapping) does not parse standalone.
#[test]
fn raw_emitted_javascript_does_not_parse_standalone() {
    if !node_available() {
        eprintln!(
            "node executable not found; skipping raw_emitted_javascript_does_not_parse_standalone"
        );
        return;
    }

    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");

    // Use 'study' template as the example
    let inputs = inputs_for_template("study");
    let inputs_json = serde_json::to_string(&inputs).expect("Failed to serialize inputs");

    // run init
    let mut cmd = aidp_with_fixture_skill(&ws);
    cmd.arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg("study")
        .arg("--no-vault")
        .arg("--workspace-root")
        .arg(".")
        .arg("--inputs")
        .arg(&inputs_json);

    let output = cmd.output().expect("Failed to run aidp run init");
    assert!(output.status.success(), "run init should succeed");

    let run_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // workflow emit — write with a `.mjs` extension so node always treats it
    // as an ES module (top-level `return` is then illegal there regardless
    // of the runner's own module-detection default), matching what the raw
    // script actually is.
    let out_path = tmpdir.path().join("raw.mjs");
    let mut cmd = aidp_with_fixture_skill(&ws);
    cmd.arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(&out_path);

    let output = cmd.output().expect("Failed to run workflow emit");
    assert!(output.status.success(), "workflow emit should succeed");

    // Run node --check on the raw script (without wrapping)
    let output = std::process::Command::new("node")
        .arg("--check")
        .arg(&out_path)
        .output()
        .expect("Failed to run node --check on raw script");

    assert!(
        !output.status.success(),
        "raw script should fail node --check"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Illegal return statement") || stderr.contains("return"),
        "node stderr should mention 'Illegal return statement': {}",
        stderr
    );
}

/// Test the minimal template: should have HELPERS + GUARD_JS, no other blocks.
/// Also generates the minimal_script.snap insta snapshot.
#[test]
fn minimal_script_golden() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");

    // run init + workflow emit for t05-minimal
    let fixture_path = ws.child("t05-minimal.yaml");
    let fixture_src = crate_dir().join("tests/fixtures/t05-minimal.yaml");
    fs::copy(&fixture_src, &fixture_path).expect("Failed to copy fixture");

    let out_path = tmpdir.path().join("minimal.js");
    let run_and_emit = try_run_init_and_emit(
        aidp,
        &ws,
        &fixture_path.display().to_string(),
        None,
        &out_path,
    )
    .expect("run init and emit should succeed");

    let script = run_and_emit.script;
    let run_id = run_and_emit.run_id;

    // Redact the run id and workspace paths for portability — see `redact_script`.
    let mut redacted = redact_script(&script, &run_id);

    // Redact run IDs for snapshot portability: replace the actual run ID with a placeholder
    // The run_id format is: YYYYMMDD-HHMMSS-XXXX (date-time-hex4)
    // We replace the actual run ID with <RUN_ID> wherever it appears
    redacted = redacted.replace(&run_id, "<RUN_ID>");

    // Check for HELPERS and GUARD_JS
    assert!(
        redacted.contains("function up("),
        "t05-minimal should contain 'function up(' (HELPERS)"
    );
    assert!(
        redacted.contains("const __GP = ["),
        "t05-minimal should contain 'const __GP = [' (GUARD_JS)"
    );

    // Regression guard: the guard pattern table's `\s` regex literals must
    // survive path redaction intact. A reintroduction of the old blanket
    // `replace("\\", "/")` would silently corrupt them to `/s` — and, since
    // insta compares two equally-corrupted sides, the snapshot test alone
    // would not catch it.
    assert!(
        redacted.contains(r"\s"),
        "path redaction must not corrupt the GUARD_JS `\\s` regex literals into `/s`"
    );

    // Check that other blocks are NOT present
    assert!(
        !redacted.contains("function bud("),
        "t05-minimal should NOT contain 'function bud(' (RETRY_HELPERS)"
    );
    assert!(
        !redacted.contains("function shards("),
        "t05-minimal should NOT contain 'function shards(' (FANOUT_HELPERS)"
    );
    assert!(
        !redacted.contains("Counting semaphore"),
        "t05-minimal should NOT contain 'Counting semaphore' (PLAN_HELPERS)"
    );
    assert!(
        !redacted.contains("function pool("),
        "t05-minimal should NOT contain 'function pool(' (POOL_HELPERS)"
    );

    // Take snapshot
    insta::assert_snapshot!(
        "minimal_script",
        prompt_examples::externalize_prompts(&redacted, "minimal")
    );
}

/// Test the retry template: should have RETRY_HELPERS, unlike minimal.
#[test]
fn retry_template_has_retry_helpers() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");

    // run init + workflow emit for t05-retry
    let fixture_path = ws.child("t05-retry.yaml");
    let fixture_src = crate_dir().join("tests/fixtures/t05-retry.yaml");
    fs::copy(&fixture_src, &fixture_path).expect("Failed to copy fixture");

    let out_path = tmpdir.path().join("retry.js");
    let run_and_emit = try_run_init_and_emit(
        aidp,
        &ws,
        &fixture_path.display().to_string(),
        None,
        &out_path,
    )
    .expect("run init and emit should succeed");

    let script = run_and_emit.script;

    // Check for RETRY_HELPERS (should have it)
    assert!(
        script.contains("function bud("),
        "t05-retry should contain 'function bud(' (RETRY_HELPERS)"
    );

    // Minimal should not have RETRY_HELPERS
    let fixture_path = ws.child("t05-minimal-check.yaml");
    let fixture_src = crate_dir().join("tests/fixtures/t05-minimal.yaml");
    fs::copy(&fixture_src, &fixture_path).expect("Failed to copy fixture");

    let out_path = tmpdir.path().join("minimal-check.js");
    let run_and_emit = try_run_init_and_emit(
        aidp,
        &ws,
        &fixture_path.display().to_string(),
        None,
        &out_path,
    )
    .expect("run init and emit should succeed");

    let script = run_and_emit.script;

    assert!(
        !script.contains("function bud("),
        "t05-minimal should NOT contain 'function bud(' (RETRY_HELPERS)"
    );
}

/// Test that knowledge-init has both FANOUT_HELPERS and POOL_HELPERS.
#[test]
fn knowledge_init_has_fanout_and_pool_helpers() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");

    // Prepare inputs for knowledge-init
    let inputs = inputs_for_template("knowledge-init");
    let inputs_json = serde_json::to_string(&inputs).expect("Failed to serialize inputs");

    let out_path = tmpdir.path().join("knowledge-init.js");
    let run_and_emit = run_init_and_emit(&ws, "knowledge-init", Some(&inputs_json), &out_path);

    let script = run_and_emit.script;

    assert!(
        script.contains("function shards("),
        "knowledge-init should contain 'function shards(' (FANOUT_HELPERS)"
    );
    assert!(
        script.contains("function pool("),
        "knowledge-init should contain 'function pool(' (POOL_HELPERS)"
    );
}

/// Test that audit has POOL_HELPERS but NOT FANOUT_HELPERS.
#[test]
fn audit_has_pool_not_fanout_helpers() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");

    // Prepare inputs for audit
    let inputs = inputs_for_template("audit");
    let inputs_json = serde_json::to_string(&inputs).expect("Failed to serialize inputs");

    let out_path = tmpdir.path().join("audit.js");
    let run_and_emit = run_init_and_emit(&ws, "audit", Some(&inputs_json), &out_path);

    let script = run_and_emit.script;

    assert!(
        script.contains("function pool("),
        "audit should contain 'function pool(' (POOL_HELPERS)"
    );
    assert!(
        !script.contains("function shards("),
        "audit should NOT contain 'function shards(' (FANOUT_HELPERS)"
    );
}

/// Test that dev-composed expands with all default settings injected.
#[test]
fn expand_dev_composed_reinjects_every_default_setting() {
    let ws = workspace();
    let mut cmd = aidp_with_fixture_skill(&ws);

    cmd.arg("pipeline")
        .arg("expand")
        .arg("--pipeline")
        .arg("dev-composed")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run pipeline expand");
    assert!(output.status.success(), "pipeline expand should succeed");

    let expanded_str = String::from_utf8_lossy(&output.stdout);
    let expanded: serde_yaml::Value =
        serde_yaml::from_str(&expanded_str).expect("expand output should be valid YAML");

    // Check for all default settings
    assert_eq!(
        expanded["settings"]["max_correction_iterations"], 2,
        "expanded should have max_correction_iterations: 2"
    );
    assert_eq!(
        expanded["settings"]["max_stage_retries"], 0,
        "expanded should have max_stage_retries: 0"
    );
    assert_eq!(
        expanded["settings"]["emit_effort"], false,
        "expanded should have emit_effort: false"
    );
    assert_eq!(
        expanded["settings"]["verify_scope"], true,
        "expanded should have verify_scope: true"
    );
    assert_eq!(
        expanded["settings"]["budget_floor_tokens"], 50000,
        "expanded should have budget_floor_tokens: 50000"
    );
    assert_eq!(
        expanded["settings"]["on_item_failure"], "pause",
        "expanded should have on_item_failure: pause"
    );
    assert_eq!(
        expanded["settings"]["tool_enforcement"], "off",
        "expanded should have tool_enforcement: off"
    );

    // Check that dev-composed itself has no settings line
    let dev_composed_path = skill_root().join("templates/dev-composed.yaml");
    let dev_composed_content =
        fs::read_to_string(&dev_composed_path).expect("Failed to read dev-composed.yaml");
    assert!(
        !dev_composed_content.contains("settings:"),
        "dev-composed.yaml itself should not contain a settings: block"
    );

    // Check for the seven qa__* stages from the reused qa pipeline
    let stages: Vec<String> = expanded["stages"]
        .as_sequence()
        .expect("stages should be a sequence")
        .iter()
        .map(|s| s["name"].as_str().unwrap_or("").to_string())
        .collect();

    let expected_stages = vec![
        "qa__explore",
        "qa__dim_correctness",
        "qa__dim_security",
        "qa__dim_perf",
        "qa__dim_maintainability",
        "qa__verify",
        "qa__report",
    ];

    for stage in &expected_stages {
        assert!(
            stages.iter().any(|s| s == stage),
            "expanded should contain stage '{}'",
            stage
        );
    }
}

/// Snapshot golden for knowledge-init (fan_out) workflow.js.
#[test]
fn fanout_script_golden() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");

    let inputs = inputs_for_template("knowledge-init");
    let inputs_json = serde_json::to_string(&inputs).expect("Failed to serialize inputs");

    let out_path = tmpdir.path().join("knowledge-init.js");
    let run_and_emit = run_init_and_emit(&ws, "knowledge-init", Some(&inputs_json), &out_path);
    let run_id = run_and_emit.run_id;

    // `knowledge-init.yaml` opens with a `type: pipeline` stage (`knowledge`,
    // `uses: knowledge-read`), so the first emit now stops at
    // `awaiting_composite` before ever reaching the fan-out stage this golden
    // exists to cover (`write_features`). Cross that boundary and re-emit to
    // get the script that actually contains the fan-out emission.
    let out_path2 = tmpdir.path().join("knowledge-init-pass2.js");
    let script = cross_boundary_and_reemit(&ws, &run_id, "knowledge", &out_path2);

    // Redact paths and run IDs
    let redacted = redact_script(&script, &run_id);

    insta::assert_snapshot!(
        "fanout_script",
        prompt_examples::externalize_prompts(&redacted, "knowledge-init")
    );
}

/// Snapshot golden for audit (parallel group) workflow.js.
#[test]
fn audit_script_golden() {
    let ws = workspace();
    let tmpdir = TempDir::new().expect("Failed to create tempdir");

    let inputs = inputs_for_template("audit");
    let inputs_json = serde_json::to_string(&inputs).expect("Failed to serialize inputs");

    let out_path = tmpdir.path().join("audit.js");
    let run_and_emit = run_init_and_emit(&ws, "audit", Some(&inputs_json), &out_path);

    let script = run_and_emit.script;
    let run_id = run_and_emit.run_id;

    // Redact paths and run IDs
    let redacted = redact_script(&script, &run_id);

    insta::assert_snapshot!(
        "audit_script",
        prompt_examples::externalize_prompts(&redacted, "audit")
    );
}

/// Snapshot golden for dev-composed expanded YAML.
#[test]
fn expand_dev_composed_golden() {
    let ws = workspace();
    let mut cmd = aidp_with_fixture_skill(&ws);

    cmd.arg("pipeline")
        .arg("expand")
        .arg("--pipeline")
        .arg("dev-composed")
        .arg("--workspace-root")
        .arg(".");

    let output = cmd.output().expect("Failed to run pipeline expand");
    assert!(output.status.success(), "pipeline expand should succeed");

    let expanded_str = String::from_utf8_lossy(&output.stdout).to_string();

    // Redact paths (convert backslash to forward slash for portability)
    let redacted = expanded_str.replace("\\", "/");

    // Note: expand output should not contain run IDs since it's a static expand
    insta::assert_snapshot!("expand_dev_composed", redacted);
}
