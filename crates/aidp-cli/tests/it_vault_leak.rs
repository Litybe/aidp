//! CLI end-to-end tests for the vault-leak exit code contract (LOC-46).
//!
//! This test suite proves that `aidp scope verify --run <id>` exits 4 (policy refusal) when
//! vault leaks are detected (checked BEFORE scope violations, which exit 6, a negative
//! verdict), and that `aidp run reconcile` also exits 4 on a leak. Exit 0 when neither is
//! present.
//!
//! A leak is a repo-relative path whose leading component is `Pipelines`, `Knowledge`,
//! or `.aidp/pipelines` while a vault is configured.
//!
//! Both `scope verify` and `run reconcile` recompute their report fresh from
//! `git diff`/`git status` on every call — neither reads a `vault_leaks` or
//! `scope_violations` value that a test might have hand-patched into
//! `run-state.json`. So every scenario here drives a REAL git repository:
//! `run init` seeds `.git/info/exclude` with `Pipelines/` the moment a vault
//! is configured (LOC-46's own belt-and-braces mechanism), so a plain `git
//! add .` on a vault-shaped path silently no-ops, exactly as intended. A
//! leak scenario therefore uses `git add -f` to simulate an artefact that
//! got through anyway (e.g. a pre-existing exclude gap) — the case this
//! guard exists to still catch.

use crate::common;
use common::*;
use serde_json::json;
use std::fs;

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

/// Initializes `ws` as a git repo with one seed commit, so `aidp run init`
/// (run afterwards) captures a real `base_sha` to diff against.
fn git_init_with_seed_commit(ws: &aidp_testkit::TempWorkspace) {
    git(ws.path(), &["init", "-q"]);
    git(ws.path(), &["config", "user.email", "test@example.com"]);
    git(ws.path(), &["config", "user.name", "aidp tests"]);
    fs::write(ws.child("seed.txt"), "seed").expect("Failed to write seed file");
    git(ws.path(), &["add", "."]);
    git(ws.path(), &["commit", "-q", "-m", "chore: seed"]);
}

/// Seeds a run from `fixture_name` (under `tests/fixtures/`) and returns its run_id.
fn seeded_run_from(ws: &aidp_testkit::TempWorkspace, fixture_name: &str) -> String {
    let fixture_path = ws.child(fixture_name);
    let fixture_src = crate_dir().join(format!("tests/fixtures/{fixture_name}"));
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
        "seeded_run_from({fixture_name}): run init should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Marks `stage_name` as `Success` in the run's persisted state via `run
/// reconcile`, so a later `scope verify` (which reads `state.stages` fresh
/// off disk, not off this call's result) finds a stage whose declared scope
/// applies. The overall workflow `status` here is irrelevant to `scope
/// verify` — it always recomputes its report from git, independent of
/// whatever `reconcile` last stored.
fn mark_stage_success(ws: &aidp_testkit::TempWorkspace, run_id: &str, stage_name: &str) {
    let mut cmd = aidp(ws);
    cmd.arg("run")
        .arg("reconcile")
        .arg("--run")
        .arg(run_id)
        .arg("--from")
        .arg("-")
        .arg("--workspace-root")
        .arg(".");
    cmd.write_stdin(
        json!({
            "status": "done",
            "results": { stage_name: { "status": "completed" } }
        })
        .to_string(),
    );
    let output = cmd.output().expect("Failed to run reconcile");
    assert!(
        output.status.success() || output.status.code() == Some(0),
        "mark_stage_success: reconcile should succeed with no leaks yet, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_scope_verify(ws: &aidp_testkit::TempWorkspace, run_id: &str) -> std::process::Output {
    let mut cmd = aidp(ws);
    cmd.arg("scope")
        .arg("verify")
        .arg("--run")
        .arg(run_id)
        .arg("--workspace-root")
        .arg(".");
    cmd.output().expect("Failed to run scope verify")
}

/// Case 1: a `Pipelines/...` file under a `repo_wide` stage (which would
/// otherwise accept it as in-scope) → `scope verify` exits 4, and stderr
/// names the offending path. No scope violation is raised for it.
#[test]
fn scope_verify_with_vault_leaks_only_exits_4_with_stderr() {
    if !git_available() {
        eprintln!("skipping: git not on PATH");
        return;
    }
    let ws = workspace();
    git_init_with_seed_commit(&ws);
    let run_id = seeded_run_from(&ws, "vault-leak-repo-wide.yaml");
    mark_stage_success(&ws, &run_id, "implement");

    let leak_path = ws.child("Pipelines/ai-dev-pipeline-v7/loc-46/file.md");
    fs::create_dir_all(leak_path.parent().unwrap()).unwrap();
    fs::write(&leak_path, "leak").unwrap();
    // run init already seeded `.git/info/exclude` with `Pipelines/` (LOC-46's
    // own belt-and-braces mechanism) — force-add to simulate a leak that got
    // through anyway.
    git(ws.path(), &["add", "-f", "Pipelines"]);

    let output = run_scope_verify(&ws, &run_id);
    assert_eq!(
        output.status.code(),
        Some(4),
        "scope verify: with vault leaks should exit 4 (policy refusal), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.is_empty(),
        "scope verify: stderr should not be empty when leaks present"
    );
    assert!(
        stderr.contains("Pipelines/ai-dev-pipeline-v7")
            || stderr.contains("vault")
            || stderr.contains("artefact"),
        "scope verify: stderr should name the offending path or mention vault, got: {}",
        stderr
    );
}

/// Case 2: a file outside the stage's declared (narrow) scope, with no
/// vault-shaped path anywhere → still exits 6, a negative verdict distinct
/// from a vault leak's policy refusal (proves the two categories did not
/// get merged).
#[test]
fn scope_verify_with_scope_violations_only_exits_6() {
    if !git_available() {
        eprintln!("skipping: git not on PATH");
        return;
    }
    let ws = workspace();
    git_init_with_seed_commit(&ws);
    // t05-minimal's "only" stage has no `scope:` — defaults to
    // `stage_dir_only`, so anything outside its stage dir is a violation.
    let run_id = seeded_run_from(&ws, "t05-minimal.yaml");
    mark_stage_success(&ws, &run_id, "only");

    let out_of_scope = ws.child("src/OutOfScope.rs");
    fs::create_dir_all(out_of_scope.parent().unwrap()).unwrap();
    fs::write(&out_of_scope, "// not in any declared scope").unwrap();

    let output = run_scope_verify(&ws, &run_id);
    assert_eq!(
        output.status.code(),
        Some(6),
        "scope verify: with scope violations but no vault leaks should exit 6 (negative verdict), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Case 3: both a vault leak and a scope violation → exits 4 (leak wins).
#[test]
fn scope_verify_with_both_exits_4_leak_wins() {
    if !git_available() {
        eprintln!("skipping: git not on PATH");
        return;
    }
    let ws = workspace();
    git_init_with_seed_commit(&ws);
    let run_id = seeded_run_from(&ws, "t05-minimal.yaml");
    mark_stage_success(&ws, &run_id, "only");

    let out_of_scope = ws.child("src/OutOfScope.rs");
    fs::create_dir_all(out_of_scope.parent().unwrap()).unwrap();
    fs::write(&out_of_scope, "// not in any declared scope").unwrap();

    let leak_path = ws.child("Pipelines/ai-dev-pipeline-v7/loc-46/file.md");
    fs::create_dir_all(leak_path.parent().unwrap()).unwrap();
    fs::write(&leak_path, "leak").unwrap();
    git(ws.path(), &["add", "-f", "Pipelines"]);

    let output = run_scope_verify(&ws, &run_id);
    assert_eq!(
        output.status.code(),
        Some(4),
        "scope verify: with both leaks and violations should exit 4 (leak wins), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Case 4: clean run-state, no git changes → exits 0.
#[test]
fn scope_verify_clean_run_state_exits_0() {
    let ws = workspace();
    let run_id = seeded_run_from(&ws, "t05-minimal.yaml");

    // No git repo, no patching — nothing to diff, nothing to flag.
    let output = run_scope_verify(&ws, &run_id);
    assert_eq!(
        output.status.code(),
        Some(0),
        "scope verify: clean run-state should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Case 5: `run reconcile` on a workspace where a `Pipelines/<repo>/...` file
/// was force-added (bypassing the exclude seeding) and a vault is configured
/// → exits 4.
#[test]
fn run_reconcile_with_vault_leak_file_exits_4() {
    if !git_available() {
        eprintln!("skipping run_reconcile_with_vault_leak_file_exits_4: git not on PATH");
        return;
    }

    let ws = workspace();
    git_init_with_seed_commit(&ws);

    // Seed a run with a vault configured (AIDP_VAULT is set by `aidp()` for
    // every command in this suite).
    let run_id = seeded_run_from(&ws, "t05-minimal.yaml");

    // Write a vault-shaped file to the repo tree AFTER base_sha, and
    // force-add it past the exclude rule `run init` just seeded.
    let vault_leak_path = ws.child("Pipelines/ai-dev-pipeline-v7/loc-46/file.md");
    fs::create_dir_all(vault_leak_path.parent().unwrap()).expect("Failed to create directory");
    fs::write(&vault_leak_path, "leak").expect("Failed to write leak file");
    git(ws.path(), &["add", "-f", "Pipelines"]);

    // Run reconcile
    let result_file = ws.child("result.json");
    fs::write(
        &result_file,
        json!({ "status": "done", "results": {} }).to_string(),
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
        Some(4),
        "run reconcile: with a Pipelines/ file and vault configured should exit 4 (policy refusal), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.is_empty(),
        "run reconcile: stderr should explain the vault leak"
    );
}
