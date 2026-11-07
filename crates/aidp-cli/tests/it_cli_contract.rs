//! End-to-end contract of the CLI argument surface: global flags, typed values,
//! bounded numbers, exclusive flags, NDJSON output and gate validation.

use crate::common;
use aidp_core::ExitStatus;
use aidp_testkit::TempWorkspace;
use common::{aidp, aidp_with_fixture_skill, workspace};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const POSSIBLE_FORMATS: &str = "[possible values: json-compact, json-pretty, ndjson, text, footer]";
const UNKNOWN_GATE: &str = "nexistepas";
const RUN_ID_PLACEHOLDER: &str = "<RUN_ID>";
const RUN_DIR_PREFIX: &str = "- run_dir: ";
const RUN_DIR_PLACEHOLDER: &str = "- run_dir: <RUN_DIR>";
const SUMMARY_TEXT_SNAPSHOT: &str = "run_summary_text";

struct InitializedRun {
    run_id: String,
    vault: PathBuf,
    workspace_root: PathBuf,
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn usage_code() -> Option<i32> {
    Some(ExitStatus::Usage.code() as i32)
}

fn init_git_repo(root: &Path) {
    let status = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(root)
        .status()
        .expect("git init should run");
    assert!(status.success(), "git init should succeed");
}

fn initialized_run(ws: &TempWorkspace) -> InitializedRun {
    let vault = ws.child("explicit-vault");
    fs::create_dir_all(vault.join(".obsidian")).expect("create vault marker");
    let workspace_root = ws.child("repo");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    init_git_repo(&workspace_root);

    let output = aidp_with_fixture_skill(ws)
        .args(["run", "init", "--pipeline", "dev", "--target", "LOC-39"])
        .arg("--vault")
        .arg(&vault)
        .arg("--workspace-root")
        .arg(&workspace_root)
        .output()
        .expect("run init should execute");
    assert!(output.status.success(), "run init: {}", stderr_of(&output));

    InitializedRun {
        run_id: stdout_of(&output).trim().to_string(),
        vault,
        workspace_root,
    }
}

fn run_state_after_verb(ws: &TempWorkspace, run: &InitializedRun) -> Output {
    aidp_with_fixture_skill(ws)
        .args(["run", "state", "--run", &run.run_id])
        .arg("--vault")
        .arg(&run.vault)
        .arg("--workspace-root")
        .arg(&run.workspace_root)
        .output()
        .expect("run state should execute")
}

fn run_state_before_verb(ws: &TempWorkspace, run: &InitializedRun) -> Output {
    aidp_with_fixture_skill(ws)
        .arg("--vault")
        .arg(&run.vault)
        .arg("--workspace-root")
        .arg(&run.workspace_root)
        .args(["run", "state", "--run", &run.run_id])
        .output()
        .expect("run state should execute")
}

/// Replaces the run id and the run directory, the only volatile parts of the text summary.
fn normalize_summary_text(text: &str, run_id: &str) -> String {
    text.replace(run_id, RUN_ID_PLACEHOLDER)
        .lines()
        .map(|line| {
            if line.starts_with(RUN_DIR_PREFIX) {
                RUN_DIR_PLACEHOLDER
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

fn run_summary(ws: &TempWorkspace, run: &InitializedRun, format: Option<&str>) -> Output {
    let mut cmd = aidp_with_fixture_skill(ws);
    cmd.args(["run", "summary", "--run", &run.run_id])
        .arg("--vault")
        .arg(&run.vault)
        .arg("--workspace-root")
        .arg(&run.workspace_root);
    if let Some(format) = format {
        cmd.args(["--format", format]);
    }
    cmd.output().expect("run summary should execute")
}

fn assert_every_line_is_json(stdout: &str) {
    assert!(!stdout.trim().is_empty(), "stdout should not be empty");
    for line in stdout.lines() {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|e| panic!("line is not JSON ({e}): {line}"));
        assert!(!line.starts_with("  "), "indented line in NDJSON: {line}");
    }
}

fn find_run_dir(dir: &Path, run_id: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().is_some_and(|name| name == run_id) {
            return Some(path);
        }
        if let Some(found) = find_run_dir(&path, run_id) {
            return Some(found);
        }
    }
    None
}

#[test]
fn a_run_written_in_an_explicit_vault_is_read_back_with_the_same_vault() {
    let ws = workspace();
    let run = initialized_run(&ws);

    let output = run_state_after_verb(&ws, &run);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("| plan |"));
    assert!(!run.workspace_root.join(".aidp").join("pipelines").exists());
}

#[test]
fn global_flags_are_accepted_before_and_after_the_verb() {
    let ws = workspace();
    let run = initialized_run(&ws);

    let before = run_state_before_verb(&ws, &run);
    let after = run_state_after_verb(&ws, &run);

    assert!(before.status.success(), "{}", stderr_of(&before));
    assert!(after.status.success(), "{}", stderr_of(&after));
    assert_eq!(before.stdout, after.stdout);
}

#[test]
fn an_invalid_track_is_refused_before_any_write_and_a_valid_one_is_stored() {
    let ws = workspace();
    let vault = ws.child("fresh-vault");
    fs::create_dir_all(&vault).expect("create vault");
    let new_ticket = |track: &str| {
        aidp(&ws)
            .args(["ticket", "new", "--vault"])
            .arg(&vault)
            .args(["--service", "cheat-profile-service", "--summary", "essai"])
            .args(["--track", track, "--allow-unknown-service"])
            .output()
            .expect("ticket new should execute")
    };

    let refused = new_ticket("bogus");

    assert_eq!(refused.status.code(), usage_code());
    assert!(stderr_of(&refused).contains("[possible values: quick, standard, deep, bug]"));
    assert!(!vault.join("TODO").join("cheat-profile-service").exists());

    let accepted = new_ticket("deep");

    assert!(accepted.status.success(), "{}", stderr_of(&accepted));
    let note = fs::read_dir(vault.join("TODO").join("cheat-profile-service"))
        .expect("service dir should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension().is_some_and(|ext| ext == "md")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !name.starts_with('_'))
        })
        .expect("a ticket note should be written");
    let content = fs::read_to_string(note).expect("read ticket note");
    assert!(content.contains("track: deep"), "{content}");
}

#[test]
fn out_of_range_git_outcomes_bounds_are_refused() {
    let ws = workspace();
    init_git_repo(ws.path());

    let overlap = aidp(&ws)
        .args(["git", "outcomes", "--overlap", "42"])
        .output()
        .expect("git outcomes should execute");
    let window = aidp(&ws)
        .args(["git", "outcomes", "--window", "0"])
        .output()
        .expect("git outcomes should execute");

    assert_eq!(overlap.status.code(), usage_code());
    assert!(stderr_of(&overlap).contains("0..=1"));
    assert_eq!(window.status.code(), usage_code());
}

#[test]
fn ndjson_output_is_line_delimited_json() {
    let ws = workspace();
    let knowledge_dir = ws.child("knowledge");
    fs::create_dir_all(&knowledge_dir).expect("create knowledge dir");
    fs::write(knowledge_dir.join("architecture.md"), "# Architecture\n").expect("write note");

    let stats = aidp(&ws)
        .args(["knowledge", "stats", "--knowledge-dir"])
        .arg(&knowledge_dir)
        .args(["--format", "ndjson"])
        .output()
        .expect("knowledge stats should execute");
    let status = aidp(&ws)
        .args(["vault", "status", "--format", "ndjson"])
        .output()
        .expect("vault status should execute");

    assert!(stats.status.success(), "{}", stderr_of(&stats));
    assert_every_line_is_json(&stdout_of(&stats));
    assert!(status.status.success(), "{}", stderr_of(&status));
    assert_every_line_is_json(&stdout_of(&status));
}

#[test]
fn run_summary_is_machine_readable_and_text_keeps_the_default_rendering() {
    let ws = workspace();
    let run = initialized_run(&ws);

    let compact = run_summary(&ws, &run, Some("json-compact"));
    let text = run_summary(&ws, &run, Some("text"));
    let default = run_summary(&ws, &run, None);

    assert!(compact.status.success(), "{}", stderr_of(&compact));
    let compact_out = stdout_of(&compact);
    assert_eq!(compact_out.trim().lines().count(), 1);
    let value: serde_json::Value =
        serde_json::from_str(compact_out.trim()).expect("summary should be JSON");
    assert!(value["counts"]["total"].as_u64().unwrap_or(0) > 0);
    assert_eq!(text.stdout, default.stdout);
    insta::assert_snapshot!(
        SUMMARY_TEXT_SNAPSHOT,
        normalize_summary_text(&stdout_of(&text), &run.run_id)
    );
    assert!(stdout_of(&text).starts_with("# Summary"));
}

#[test]
fn knowledge_stale_works_on_a_fresh_vault_and_rejects_the_removed_flag() {
    let ws = workspace();
    let knowledge_dir = ws.child("fresh").join("Knowledge").join("somerepo");

    let stale = aidp(&ws)
        .args(["knowledge", "stale", "--knowledge-dir"])
        .arg(&knowledge_dir)
        .output()
        .expect("knowledge stale should execute");
    let with_since = aidp(&ws)
        .args([
            "knowledge",
            "stale",
            "--since",
            "2026-01-01",
            "--knowledge-dir",
        ])
        .arg(&knowledge_dir)
        .output()
        .expect("knowledge stale should execute");

    assert!(stale.status.success(), "{}", stderr_of(&stale));
    let cache = fs::read_to_string(knowledge_dir.join(".aidp-stale.json"))
        .expect("stale cache should be written");
    serde_json::from_str::<serde_json::Value>(&cache).expect("stale cache should be JSON");
    assert_eq!(with_since.status.code(), usage_code());
}

#[test]
fn an_unknown_gate_is_refused_before_any_emission() {
    let ws = workspace();
    let run = initialized_run(&ws);
    let approve = |gate: &str| {
        aidp_with_fixture_skill(&ws)
            .args(["gate", "approve", "--run", &run.run_id, "--gate", gate])
            .arg("--vault")
            .arg(&run.vault)
            .arg("--workspace-root")
            .arg(&run.workspace_root)
            .output()
            .expect("gate approve should execute")
    };
    let run_dir = find_run_dir(&run.vault.join("Pipelines"), &run.run_id)
        .expect("run dir should exist in the vault");
    let workflow_js = run_dir.join("workflow.js");
    let _ = fs::remove_file(&workflow_js);

    let refused = approve(UNKNOWN_GATE);

    assert!(!refused.status.success());
    let stderr = stderr_of(&refused);
    assert!(stderr.contains(UNKNOWN_GATE), "{stderr}");
    assert!(stderr.contains("plan"), "{stderr}");
    assert!(!workflow_js.exists());

    let accepted = approve("plan");

    assert!(accepted.status.success(), "{}", stderr_of(&accepted));
    let envelope: serde_json::Value =
        serde_json::from_str(stdout_of(&accepted).trim()).expect("envelope should be JSON");
    for key in ["scriptPath", "resumeFromRunId", "args"] {
        assert!(envelope.get(key).is_some(), "missing key {key}: {envelope}");
    }
}

#[test]
fn mutually_exclusive_flags_are_refused_by_clap() {
    let ws = workspace();

    let validate = aidp_with_fixture_skill(&ws)
        .args(["validate", "--all", "--inputs", "{}"])
        .output()
        .expect("validate should execute");
    let route = aidp(&ws)
        .args(["ticket", "route", "--from-unrouted", "--service", "x"])
        .output()
        .expect("ticket route should execute");

    assert_eq!(validate.status.code(), usage_code());
    assert!(stderr_of(&validate).contains("--all"));
    assert!(stderr_of(&validate).contains("--inputs"));
    assert_eq!(route.status.code(), usage_code());
}

#[test]
fn format_documents_its_possible_values_and_rejects_others_alike() {
    let ws = workspace();

    for help_path in [vec!["doctor", "-h"], vec!["ticket", "list", "-h"]] {
        let help = aidp(&ws)
            .args(&help_path)
            .output()
            .expect("help should run");
        assert!(
            stdout_of(&help).contains(POSSIBLE_FORMATS),
            "{help_path:?} lacks the possible values"
        );
    }

    let doctor = aidp(&ws)
        .args(["doctor", "--format", "bogus"])
        .output()
        .expect("doctor should execute");
    let list = aidp(&ws)
        .args(["ticket", "list", "--format", "bogus"])
        .output()
        .expect("ticket list should execute");

    assert_eq!(doctor.status.code(), usage_code());
    assert_eq!(list.status.code(), usage_code());
    for output in [&doctor, &list] {
        assert!(stderr_of(output).contains("invalid value 'bogus' for '--format <FORMAT>'"));
    }
}
