#![allow(dead_code)]

pub mod gh_shim;

use aidp_testkit::TempWorkspace;
use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;

/// A minimal but schema-valid prices grid. An invalid grid would not panic the
/// child (`cost::load_error()` reports it and `run reconcile` degrades usage to
/// unavailable); tests wanting a priced run need it to parse.
const MINIMAL_PRICES_JSON: &str = r#"{"pricing_version":"test","tiers":{}}"#;

pub const COMMAND_STARTED: &str = "command started";
pub const AIDP_LOG_VAR: &str = "AIDP_LOG";
pub const FORMAT_FLAG: &str = "--format";
pub const FORMAT_JSON: &str = "json-compact";
pub const WORKSPACE_FLAG: &str = "--workspace-root";
pub const RUN_ARG: &str = "run";
pub const INIT_ARG: &str = "init";
pub const PIPELINE_FLAG: &str = "--pipeline";
pub const TARGET_FLAG: &str = "--target";
pub const INPUTS_FLAG: &str = "--inputs";
pub const WORKFLOW_ARG: &str = "workflow";
pub const EMIT_ARG: &str = "emit";
pub const RUN_FLAG: &str = "--run";
pub const OUT_FLAG: &str = "--out";
pub const STATE_ARG: &str = "state";
pub const PLAN_STATUS_ARG: &str = "plan-status";
const RUN_ID_DATE_LEN: usize = 8;
const RUN_ID_TIME_LEN: usize = 6;
const RUN_ID_PARTS: usize = 3;
const RUN_ID_MIN_LEN: usize = 15;

/// Tells whether a line has the `YYYYMMDD-HHMMSS-suffix` shape of a run id.
pub fn is_run_id(line: &str) -> bool {
    let parts: Vec<&str> = line.split('-').collect();
    line.len() >= RUN_ID_MIN_LEN
        && parts.len() == RUN_ID_PARTS
        && parts[0].len() == RUN_ID_DATE_LEN
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].len() == RUN_ID_TIME_LEN
        && parts[1].chars().all(|c| c.is_ascii_digit())
        && !parts[2].is_empty()
        && parts[2]
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
}

/// Creates a new temporary workspace for testing.
///
/// This thin wrapper around `TempWorkspace::new("cli-surface")` provides a
/// hermetic environment for running `aidp` binary integration tests.
pub fn workspace() -> TempWorkspace {
    TempWorkspace::new("cli-surface")
}

/// Builds a command to run the `aidp` binary in a hermetic environment.
///
/// The command is configured to:
/// - Run in the temporary workspace directory
/// - Use isolated HOME, USERPROFILE, AIDP_VAULT, and other directories
/// - Block access to the developer's real machine via AIDP_GATEWAY_URL
/// - Remove environment variables that could leak machine state
pub fn aidp(ws: &TempWorkspace) -> Command {
    // Create necessary directories if they don't exist
    let home_dir = ws.child("home");
    let vault_dir = ws.child("vault");
    let skills_dir = ws.child("skills");
    let claude_projects_dir = ws.child("claude-projects");
    let prices_file = ws.child("prices.json");

    let _ = fs::create_dir_all(&home_dir);
    let _ = fs::create_dir_all(&vault_dir);
    let _ = fs::create_dir_all(&skills_dir);
    let _ = fs::create_dir_all(&claude_projects_dir);

    let _ = fs::write(&prices_file, MINIMAL_PRICES_JSON);

    // Build the command
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aidp"));
    cmd.current_dir(ws.path());

    // Set environment variables on the CHILD process only.
    // Using Command::env / Command::env_remove on the child ensures
    // we don't serialize tests with a process-wide mutex lock like EnvGuard does.

    // Set HOME and USERPROFILE to isolated directories.
    // aidp_core::vault::home_dir() reads USERPROFILE first and only falls back
    // to HOME, so we must set both to prevent the real Windows profile from
    // being reachable when USERPROFILE is not explicitly set.
    cmd.env("HOME", home_dir.to_string_lossy().to_string());
    cmd.env("USERPROFILE", home_dir.to_string_lossy().to_string());

    // Set AIDP-specific paths to isolated directories
    cmd.env("AIDP_VAULT", vault_dir.to_string_lossy().to_string());
    cmd.env("AIDP_SKILL_ROOT", skills_dir.to_string_lossy().to_string());
    cmd.env(
        "AIDP_CLAUDE_PROJECTS_DIR",
        claude_projects_dir.to_string_lossy().to_string(),
    );
    cmd.env(
        "AIDP_MODEL_PRICES",
        prices_file.to_string_lossy().to_string(),
    );

    // Block the gateway with a non-routable address that fails fast
    cmd.env("AIDP_GATEWAY_URL", "http://127.0.0.1:1");

    // Remove or blank environment variables that could leak machine state
    cmd.env_remove("AIDP_RUN");
    cmd.env_remove("AIDP_STAGE");
    cmd.env_remove("AIDP_AUTO_APPROVE_GATES");
    cmd.env_remove("AIDP_HOOK_TOKEN");

    // Clear Sonar variables so aidp_integrations::sonar::resolve_config does not read
    // a developer machine's exported tokens or host URLs, which would pollute cascade
    // levels 3, 4 and 5 (environment, home config, properties file).
    cmd.env_remove("SONAR_HOST_URL");
    cmd.env_remove("SONAR_PROJECT_KEY");
    cmd.env_remove("SONAR_ORGANIZATION");
    cmd.env_remove("SONAR_TOKEN");
    cmd.env_remove("AIDP_SONAR_TOKEN");

    cmd
}

/// List of all 33 subcommand names in the expected order.
pub const SUBCOMMANDS: [&str; 22] = [
    "validate",
    "pipeline",
    "run",
    "scope",
    "agents",
    "skills",
    "topo",
    "progress",
    "gate",
    "gh",
    "mock",
    "workflow",
    "vault",
    "knowledge",
    "gateway",
    "ticket",
    "doctor",
    "status",
    "setup",
    "cost",
    "git",
    "findings",
];

/// Normalize command output to make it portable between Windows and Linux.
///
/// This function calls `aidp_testkit::normalize()` to replace workspace paths,
/// run IDs, and backslashes, then normalizes the binary name token so that
/// both `aidp.exe` and `aidp` variants resolve to `<BIN>`.
///
/// The binary name normalization is necessary because clap derives the `Usage:`
/// line from `argv[0]` (not from `#[command(name = "aidp")]`), so the same
/// binary prints `Usage: aidp.exe ...` on Windows and `Usage: aidp ...` on Linux.
pub fn normalize_output(raw: &str, ws: &TempWorkspace) -> String {
    // First apply the testkit's standard normalization
    let normalized = aidp_testkit::normalize(raw, ws);

    // Then normalize the binary name token
    normalize_binary_name(&normalized)
}

/// Check if a byte is a word boundary in the binary name context.
fn is_word_boundary(b: Option<u8>) -> bool {
    matches!(
        b,
        None | Some(b' ') | Some(b'\n') | Some(b'\'') | Some(b'`') | Some(b'>')
    )
}

/// Normalize occurrences of the binary name to `<BIN>`.
///
/// Replaces both `aidp.exe` and `aidp` when they appear as the invoked program name
/// (after `Usage: `, in error suggestions, or in other command contexts).
/// Only matches in actual command positions: after "Usage: ", after quotes, or at start of line.
pub fn normalize_binary_name(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Try to match "aidp.exe" first (more specific)
        if bytes[i..].starts_with(b"aidp.exe") {
            // Check if it's a whole-word occurrence (preceded by space, newline, or start)
            let is_start =
                i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\n' || bytes[i - 1] == b':';
            if is_start {
                // Also check that it's followed by whitespace or end of line (word boundary)
                let after_idx = i + 8;
                let is_end = is_word_boundary(bytes.get(after_idx).copied());
                if is_end {
                    result.push_str("<BIN>");
                    i += 8;
                    continue;
                }
            }
        }

        // Try to match " aidp" but only in command contexts (not after regular letters)
        if i > 0 && bytes[i - 1] == b' ' && bytes[i..].starts_with(b"aidp") {
            // Only match if the space is in a command context:
            // - preceded by ":" (as in "Usage: aidp")
            // - preceded by a quote (as in "'aidp")
            // - at start of line (if preceded by newline)
            let is_command_context = if i >= 2 {
                let ch_before_space = bytes[i - 2];
                ch_before_space == b':' || ch_before_space == b'\'' || ch_before_space == b'`'
            } else {
                // i == 1, space is second character (after first char at position 0)
                true
            };

            if is_command_context {
                // Check that after "aidp" is a word boundary
                let after_idx = i + 4;
                let is_end = is_word_boundary(bytes.get(after_idx).copied());
                if is_end {
                    result.push_str("<BIN>");
                    i += 4;
                    continue;
                }
            }
        }

        // Try to match 'aidp' or `aidp` (quote directly before aidp, as in "try 'aidp")
        if i > 0
            && (bytes[i - 1] == b'\'' || bytes[i - 1] == b'`')
            && bytes[i..].starts_with(b"aidp")
        {
            // Check that after "aidp" is a word boundary
            let after_idx = i + 4;
            let is_end = is_word_boundary(bytes.get(after_idx).copied());
            if is_end {
                result.push_str("<BIN>");
                i += 4;
                continue;
            }
        }

        // Try to match "Usage: aidp" (right after a colon with space)
        if i > 0 && bytes[i - 1] == b':' && bytes[i..].starts_with(b" aidp") {
            // Skip the space
            if i + 5 <= bytes.len() {
                let after_idx = i + 5;
                let is_end = after_idx >= bytes.len()
                    || bytes[after_idx] == b' '
                    || bytes[after_idx] == b'\n'
                    || bytes[after_idx] == b'\''
                    || bytes[after_idx] == b'`'
                    || bytes[after_idx] == b'>';
                if is_end {
                    result.push(' ');
                    result.push_str("<BIN>");
                    i += 5;
                    continue;
                }
            }
        }

        // No match, copy the current character
        if let Some(ch) = input[i..].chars().next() {
            result.push(ch);
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }

    result
}

/// The frozen skill the tests compile: `fixtures/skill` of the workspace, never the real skill.
pub fn skill_root() -> PathBuf {
    aidp_testkit::fixtures::skill_root()
}

/// This crate's directory, for its own fixtures under `tests/fixtures`.
pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Builds a command to run `aidp` with access to the frozen skill's templates.
///
/// Unlike `aidp()`, which uses an empty skills directory to keep HOME/USERPROFILE
/// hermetic, this variant overrides AIDP_SKILL_ROOT to point at the frozen skill. This is necessary for tests of `aidp validate --all` and
/// `aidp run init --pipeline <shipped template>`, which resolve templates through
/// the skill root.
///
/// HOME/USERPROFILE remain pointed at the temporary workspace, ensuring that
/// emitted workflow.js files are snapshot-portable (no absolute machine paths).
pub fn aidp_with_fixture_skill(ws: &TempWorkspace) -> Command {
    let mut cmd = aidp(ws);
    cmd.env(
        "AIDP_SKILL_ROOT",
        skill_root().to_string_lossy().to_string(),
    );
    cmd
}

/// Result of a run init + workflow emit sequence.
pub struct EmittedRun {
    pub run_id: String,
    pub script: String,
}

/// Run `aidp run init --pipeline <pipeline> --no-vault --workspace-root . [--inputs <inputs>]`
/// then `aidp workflow emit --run <run_id> --workspace-root . --out <out_path>`, using the
/// command builder `builder` for both, and return the run id plus the emitted script contents.
/// Returns Err(String) with a human-readable message when either step fails.
pub fn try_run_init_and_emit(
    builder: fn(&TempWorkspace) -> Command,
    ws: &TempWorkspace,
    pipeline: &str,
    inputs: Option<&str>,
    out_path: &std::path::Path,
) -> Result<EmittedRun, String> {
    // Step 1: run init
    let mut cmd = builder(ws);
    cmd.arg("run")
        .arg("init")
        .arg("--pipeline")
        .arg(pipeline)
        .arg("--no-vault")
        .arg("--workspace-root")
        .arg(".");

    if let Some(inputs_str) = inputs {
        cmd.arg("--inputs").arg(inputs_str);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("run init failed to execute: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("run init failed: {}", stderr));
    }

    let run_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if run_id.is_empty() {
        return Err("run init produced empty run_id".to_string());
    }

    // Step 2: workflow emit
    let mut cmd = builder(ws);
    cmd.arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(&run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(out_path);

    let output = cmd
        .output()
        .map_err(|e| format!("workflow emit failed to execute: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("workflow emit failed: {}", stderr));
    }

    // Step 3: read the emitted file
    let script = fs::read_to_string(out_path)
        .map_err(|e| format!("failed to read emitted script at {:?}: {}", out_path, e))?;

    Ok(EmittedRun { run_id, script })
}

/// Cross a composite (`type: pipeline`) boundary on an existing run
/// (`aidp run record-resume --crossed-boundary <composite>`) then re-emit
/// (`aidp workflow emit`), returning the new script. Mirrors
/// `try_run_init_and_emit`'s two-step CLI-subprocess pattern, but for a run
/// that already exists and whose first `emit` stopped at `awaiting_composite`.
pub fn try_cross_boundary_and_reemit(
    builder: fn(&TempWorkspace) -> Command,
    ws: &TempWorkspace,
    run_id: &str,
    composite: &str,
    out_path: &std::path::Path,
) -> Result<String, String> {
    let mut cmd = builder(ws);
    cmd.arg("run")
        .arg("record-resume")
        .arg("--run")
        .arg(run_id)
        .arg("--crossed-boundary")
        .arg(composite)
        .arg("--workspace-root")
        .arg(".");

    let output = cmd
        .output()
        .map_err(|e| format!("run record-resume failed to execute: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("run record-resume failed: {}", stderr));
    }

    let mut cmd = builder(ws);
    cmd.arg("workflow")
        .arg("emit")
        .arg("--run")
        .arg(run_id)
        .arg("--workspace-root")
        .arg(".")
        .arg("--out")
        .arg(out_path);

    let output = cmd
        .output()
        .map_err(|e| format!("workflow emit failed to execute: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("workflow emit failed: {}", stderr));
    }

    fs::read_to_string(out_path)
        .map_err(|e| format!("failed to read emitted script at {:?}: {}", out_path, e))
}

/// Same as `try_cross_boundary_and_reemit` with `aidp_with_fixture_skill`, panicking
/// with the underlying stderr on failure.
pub fn cross_boundary_and_reemit(
    ws: &TempWorkspace,
    run_id: &str,
    composite: &str,
    out_path: &std::path::Path,
) -> String {
    try_cross_boundary_and_reemit(aidp_with_fixture_skill, ws, run_id, composite, out_path)
        .unwrap_or_else(|e| panic!("{}", e))
}

/// Same as `try_run_init_and_emit` with `aidp_with_fixture_skill`, panicking with the
/// underlying stderr on failure.
pub fn run_init_and_emit(
    ws: &TempWorkspace,
    pipeline: &str,
    inputs: Option<&str>,
    out_path: &std::path::Path,
) -> EmittedRun {
    try_run_init_and_emit(aidp_with_fixture_skill, ws, pipeline, inputs, out_path)
        .unwrap_or_else(|e| panic!("{}", e))
}

/// Lists all template IDs from the templates directory.
///
/// Returns file stems of all `.yaml` files (excluding directories like `templates/legacy`),
/// sorted alphabetically. Measured: 40 template IDs.
pub fn template_ids() -> Vec<String> {
    let templates_dir = skill_root().join("templates");
    let mut ids = Vec::new();

    if let Ok(entries) = fs::read_dir(&templates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip directories; keep only files
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "yaml" {
                        if let Some(stem) = path.file_stem() {
                            if let Some(id) = stem.to_str() {
                                ids.push(id.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    ids.sort();
    ids
}

/// Derives input values for a template.
///
/// Reads `templates/<id>.yaml`, walks its top-level `inputs:` mapping, and
/// produces a value for each declared input:
/// - If the input has a `default`, use it.
/// - Else if it declares an enum (`values` list), use the first entry.
/// - Else, use a typed dummy: `"x"` for string/unset, `1` for integer/number, `true` for boolean.
///
/// Returns a `serde_json::Value` object suitable for `--inputs`.
pub fn inputs_for_template(id: &str) -> serde_json::Value {
    let template_file = skill_root().join(format!("templates/{}.yaml", id));

    // Read and parse the template YAML
    let content = fs::read_to_string(&template_file)
        .unwrap_or_else(|_| panic!("Failed to read template: {:?}", template_file));

    let parsed: serde_yaml::Value = serde_yaml::from_str(&content)
        .unwrap_or_else(|_| panic!("Failed to parse YAML template: {:?}", template_file));

    let mut inputs_obj = serde_json::json!({});

    // Extract the inputs mapping
    if let Some(inputs_map) = parsed.get("inputs").and_then(|v| v.as_mapping()) {
        for (key, input_spec) in inputs_map.iter() {
            let input_name = key.as_str().unwrap_or("unknown");

            // Try to extract the value
            let value = if let Some(default) = input_spec.get("default") {
                // Convert serde_yaml::Value to serde_json::Value
                serde_json::to_value(default).unwrap_or(serde_json::json!("x"))
            } else if let Some(values) = input_spec.get("values").and_then(|v| v.as_sequence()) {
                // Use the first entry from the enum
                if let Some(first) = values.first() {
                    serde_json::to_value(first).unwrap_or(serde_json::json!("x"))
                } else {
                    serde_json::json!("x")
                }
            } else {
                // Use a typed dummy
                match input_spec.get("kind").and_then(|k| k.as_str()) {
                    Some("integer") | Some("number") => serde_json::json!(1),
                    Some("bool") => serde_json::json!(true),
                    _ => serde_json::json!("x"),
                }
            };

            inputs_obj[input_name] = value;
        }
    }

    inputs_obj
}

/// Wraps a workflow script for syntax checking with `node --check`.
///
/// The emitted script is a function body (with `return` at top level) preceded by
/// an ESM `export const meta`. Running `node --check` on the raw file exits 1 with
/// `SyntaxError: Illegal return statement`. This wrapper replaces the FIRST
/// occurrence of `export const meta` with `const meta` and wraps the body in an
/// async IIFE to make it valid JavaScript.
pub fn wrap_for_node_check(script: &str) -> String {
    let modified = script.replacen("export const meta", "const meta", 1);
    format!("(async function(args){{\n{}\n}});\n", modified)
}

/// Checks if the `node` executable is available and working.
///
/// Returns `true` if `node --version` runs successfully, `false` otherwise.
pub fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_aidp_exe_windows_usage() {
        let input = "Usage: aidp.exe run --help";
        let _ws = workspace();
        let output = normalize_binary_name(input);
        assert_eq!(output, "Usage: <BIN> run --help");
    }

    #[test]
    fn normalize_aidp_linux_usage() {
        let input = "Usage: aidp run --help";
        let _ws = workspace();
        let output = normalize_binary_name(input);
        assert_eq!(output, "Usage: <BIN> run --help");
    }

    #[test]
    fn normalize_preserves_aidp_in_other_contexts() {
        let input = "The aidp command is useful";
        let output = normalize_binary_name(input);
        // Should not be replaced when not in command position
        assert_eq!(output, "The aidp command is useful");
    }

    #[test]
    fn normalize_handles_multiple_occurrences() {
        let input = "Usage: aidp.exe run\nUsage: aidp validate";
        let output = normalize_binary_name(input);
        assert_eq!(output, "Usage: <BIN> run\nUsage: <BIN> validate");
    }

    #[test]
    fn normalize_in_try_suggestion() {
        let input = "try 'aidp --help' for more";
        let output = normalize_binary_name(input);
        assert_eq!(output, "try '<BIN> --help' for more");
    }

    #[test]
    fn workspace_creates_temp_dir() {
        let ws = workspace();
        assert!(ws.path().exists());
    }

    #[test]
    fn aidp_command_builds_successfully() {
        let ws = workspace();
        let _cmd = aidp(&ws);
        // Just verify it builds; actual execution is left to integration tests
        // The fact that this doesn't panic means the command was built successfully
    }

    #[test]
    fn subcommands_have_correct_count() {
        assert_eq!(SUBCOMMANDS.len(), 22);
    }

    #[test]
    fn subcommands_are_in_expected_order() {
        let expected = vec![
            "validate",
            "pipeline",
            "run",
            "scope",
            "agents",
            "skills",
            "topo",
            "progress",
            "gate",
            "gh",
            "mock",
            "workflow",
            "vault",
            "knowledge",
            "gateway",
            "ticket",
            "doctor",
            "status",
            "setup",
            "cost",
            "git",
            "findings",
        ];
        assert_eq!(SUBCOMMANDS.to_vec(), expected);
    }

    #[test]
    fn template_ids_non_empty_and_contains_required() {
        let ids = template_ids();
        assert!(!ids.is_empty(), "template_ids should not be empty");

        // Check that required templates are present
        assert!(
            ids.contains(&"dev".to_string()),
            "template_ids should contain 'dev'"
        );
        assert!(
            ids.contains(&"study".to_string()),
            "template_ids should contain 'study'"
        );
        assert!(
            ids.contains(&"audit".to_string()),
            "template_ids should contain 'audit'"
        );
        assert!(
            ids.contains(&"dev-composed".to_string()),
            "template_ids should contain 'dev-composed'"
        );
        assert!(
            ids.contains(&"knowledge-init".to_string()),
            "template_ids should contain 'knowledge-init'"
        );

        // Check that legacy is NOT included
        assert!(
            !ids.contains(&"legacy".to_string()),
            "template_ids should not contain 'legacy' directory name"
        );
    }

    #[test]
    fn template_ids_sorted() {
        let ids = template_ids();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "template_ids should return sorted list");
    }

    #[test]
    fn wrap_for_node_check_wraps_script() {
        let script = "export const meta = {};\nreturn 42;";
        let wrapped = wrap_for_node_check(script);

        // Should start with the async IIFE
        assert!(
            wrapped.starts_with("(async function(args){"),
            "wrapped script should start with (async function(args){{"
        );

        // Should contain const meta (not export const meta)
        assert!(
            wrapped.contains("const meta = {};"),
            "wrapped script should contain 'const meta = {{}}'"
        );
        assert!(
            !wrapped.contains("export const meta"),
            "wrapped script should not contain 'export const meta'"
        );

        // Should end with closing statement
        assert!(
            wrapped.ends_with("});\n"),
            "wrapped script should end with }});"
        );
    }

    #[test]
    fn wrap_for_node_check_replaces_only_first_export() {
        let script = "export const meta = {};\nexport const foo = 'bar';\nreturn 42;";
        let wrapped = wrap_for_node_check(script);

        // Should have exactly one "const meta" and no "export const meta"
        let const_meta_count = wrapped.matches("const meta").count();
        let export_const_meta_count = wrapped.matches("export const meta").count();

        assert_eq!(const_meta_count, 1, "should have exactly one 'const meta'");
        assert_eq!(
            export_const_meta_count, 0,
            "should have no 'export const meta'"
        );

        // But should preserve other "export const" statements
        assert!(
            wrapped.contains("export const foo"),
            "should preserve other export const statements"
        );
    }
}
