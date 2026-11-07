//! Regression test for LOC-46: with a vault configured, a stage that writes a
//! file at a path matching `Pipelines/<repo>/<feature>/<run>/<stage>/<file>`
//! relative to the workspace root is flagged by `reconcile` distinctly from a
//! normal `scope_violations` entry — even under a `repo_wide` stage, which
//! accepts the very same path as perfectly in-scope. That silent acceptance
//! is exactly the bug class this guard closes: five such artefacts were
//! committed into the real repo before `vault_leak::detect` existed.
//!
//! Setup idiom follows `ledger_error_reconcile.rs`: a `RunState` built
//! directly (no `aidp run init`), driven straight through
//! `aidp_core::run::reconcile`.
use std::path::PathBuf;
use std::process::Command;

use aidp_core::run::{reconcile, RunState, StageState, StageStatus};
use aidp_testkit::TempWorkspace;

const GIT_IDENTITY_ARGS: [&str; 6] = [
    "-c",
    "user.name=aidp tests",
    "-c",
    "user.email=test@example.com",
    "-c",
    "commit.gpgsign=false",
];

/// Runs git with an inline identity so no `git config` process is needed.
fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(GIT_IDENTITY_ARGS)
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git must be on PATH for this test");
    assert!(status.success(), "git {:?} failed in {:?}", args, dir);
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn fixture_pipeline_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault_leak_repo_wide.yaml")
}

/// Builds a temp git repo with one commit (`base_sha`), and a `RunState`
/// pointing at the `vault_leak_repo_wide` fixture's single `repo_wide`
/// `implement` stage, already marked `Success`.
fn seeded_state(workspace: &TempWorkspace, vault_root: Option<PathBuf>) -> RunState {
    let ws_path = workspace.path().to_path_buf();
    git(&ws_path, &["init", "-q"]);
    git(
        &ws_path,
        &["commit", "-q", "--allow-empty", "-m", "chore: seed"],
    );
    let base_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&ws_path)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    RunState {
        run_id: "r-vault-leak".into(),
        pipeline_path: fixture_pipeline_path(),
        pipeline_id: "vault-leak-repo-wide".into(),
        workspace_root: ws_path.clone(),
        run_dir: ws_path.clone(),
        started_at: "t".into(),
        stages: vec![StageState {
            name: "implement".into(),
            status: StageStatus::Success,
            stage_dir: ws_path.join("implement"),
            ..Default::default()
        }],
        vault_root,
        base_sha: Some(base_sha),
        ..Default::default()
    }
}

fn done_result() -> serde_json::Value {
    serde_json::json!({
        "status": "done",
        "results": { "implement": { "status": "completed" } }
    })
}

/// Case: vault configured, a stray `Pipelines/...` file is committed alongside
/// a legitimate source file. `reconcile` must flag the stray file as a vault
/// leak and NOT as a scope violation — the `repo_wide` stage would otherwise
/// have accepted it as in-scope.
#[test]
fn reconcile_flags_stray_vault_path_as_leak_not_violation() {
    if !git_available() {
        eprintln!("skipping: git not on PATH");
        return;
    }
    let workspace = TempWorkspace::new("vault-leak-repo-wide");
    let vault_dir = TempWorkspace::new("vault-leak-repo-wide-vault");
    let mut st = seeded_state(&workspace, Some(vault_dir.path().to_path_buf()));

    let leak_path = workspace.path().join(
        "Pipelines/ai-dev-pipeline-v7/loc-46/20260922-120724-7dd8/implement/changed-files.json",
    );
    std::fs::create_dir_all(leak_path.parent().unwrap()).unwrap();
    std::fs::write(&leak_path, "{}").unwrap();

    let legit_path = workspace.path().join("src/main.rs");
    std::fs::create_dir_all(legit_path.parent().unwrap()).unwrap();
    std::fs::write(&legit_path, "fn main() {}").unwrap();

    git(workspace.path(), &["add", "."]);
    git(
        workspace.path(),
        &[
            "commit",
            "-q",
            "-m",
            "implement: T1 (with a stray vault path)",
        ],
    );

    let outcome = reconcile(&mut st, &done_result());
    assert!(outcome.is_ok(), "reconcile must succeed: {:?}", outcome);

    assert_eq!(
        st.vault_leaks.len(),
        1,
        "exactly the stray Pipelines/ path must be flagged: {:?}",
        st.vault_leaks
    );
    assert_eq!(
        st.vault_leaks[0].kind,
        aidp_core::vault_leak::LeakKind::RunArtifact
    );
    assert!(st.vault_leaks[0].path.contains("Pipelines"));

    // The crux of the regression: under the old code, `repo_wide` would have
    // accepted the stray path as in-scope, so it must never surface here.
    assert!(
        st.scope_violations.is_empty(),
        "a vault leak is a distinct failure class, not a scope violation: {:?}",
        st.scope_violations
    );

    // The legitimate source file must not be flagged as either.
    assert!(st
        .vault_leaks
        .iter()
        .all(|l| !l.path.contains("src/main.rs")));
}

/// Case: legacy mode (no vault configured) — the same stray path is written
/// on purpose (that is what legacy mode is for) and must never be flagged.
#[test]
fn reconcile_does_not_flag_vault_shaped_paths_in_legacy_mode() {
    if !git_available() {
        eprintln!("skipping: git not on PATH");
        return;
    }
    let workspace = TempWorkspace::new("vault-leak-legacy");
    let mut st = seeded_state(&workspace, None);

    let leak_path = workspace
        .path()
        .join("Pipelines/ai-dev-pipeline-v7/loc-46/run/implement/file.md");
    std::fs::create_dir_all(leak_path.parent().unwrap()).unwrap();
    std::fs::write(&leak_path, "legacy artefact, on purpose").unwrap();
    git(workspace.path(), &["add", "."]);
    git(
        workspace.path(),
        &["commit", "-q", "-m", "implement: legacy artefact"],
    );

    let outcome = reconcile(&mut st, &done_result());
    assert!(outcome.is_ok(), "reconcile must succeed: {:?}", outcome);

    assert!(
        st.vault_leaks.is_empty(),
        "legacy mode (no vault_root) writes into the repo on purpose: {:?}",
        st.vault_leaks
    );
}
