use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use aidp_core::process_runner::OsProcessRunner;

use crate::worktrees::{
    ensure, ensure_scratch, existing_for_branch, modified_count, release_scratch, remove,
    WIP_REF_PREFIX,
};

const KEY: &str = "LOC-1";
const REPO: &str = "repo";
const SCRATCH: &str = "idea";
const SHA_PREFIX_LEN: usize = 12;

fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok()
}

fn run_git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

struct Fixture {
    _root: tempfile::TempDir,
    project: PathBuf,
    vault: PathBuf,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let origin = root.path().join("origin.git");
    let project = root.path().join("project");
    let vault = root.path().join("vault");
    for dir in [&origin, &project, &vault] {
        std::fs::create_dir_all(dir).unwrap();
    }
    run_git(&origin, &["init", "--bare", "--initial-branch=master"]);
    run_git(&project, &["init", "--initial-branch=master"]);
    run_git(&project, &["config", "user.email", "t@example.com"]);
    run_git(&project, &["config", "user.name", "t"]);
    run_git(&project, &["config", "commit.gpgsign", "false"]);
    std::fs::write(project.join("a.txt"), "a").unwrap();
    run_git(&project, &["add", "."]);
    run_git(&project, &["commit", "-m", "init"]);
    run_git(
        &project,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    run_git(&project, &["push", "origin", "master"]);
    Fixture {
        _root: root,
        project,
        vault,
    }
}

fn no_sleep(_: Duration) {}

#[test]
fn it_first_launch_creates_then_relaunch_reuses() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let first = ensure(&OsProcessRunner, &fx.project, &fx.vault, REPO, KEY, &[]).unwrap();
    assert!(!first.reused);
    assert!(first.workspace.path.join("a.txt").is_file());
    assert_eq!(
        run_git(&first.workspace.path, &["branch", "--show-current"]),
        "feat/LOC-1"
    );
    let second = ensure(&OsProcessRunner, &fx.project, &fx.vault, REPO, KEY, &[]).unwrap();
    assert!(second.reused);
    let found = existing_for_branch(&OsProcessRunner, &fx.project, &fx.vault, REPO, "feat/LOC-1");
    assert!(found.is_some());
}

#[test]
fn it_branch_checked_out_in_the_project_is_refused() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    run_git(&fx.project, &["checkout", "-b", "feat/LOC-1"]);
    let error = ensure(&OsProcessRunner, &fx.project, &fx.vault, REPO, KEY, &[]).unwrap_err();
    assert_eq!(error.slug(), "branch-checked-out-elsewhere");
}

#[test]
fn it_local_ignored_files_are_copied_but_tracked_ones_are_not() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    std::fs::create_dir_all(fx.project.join(".claude")).unwrap();
    std::fs::write(fx.project.join(".claude/settings.local.json"), "{}").unwrap();
    std::fs::write(fx.project.join(".env"), "SECRET=1").unwrap();
    let copy = vec![
        ".claude/settings.local.json".to_string(),
        "a.txt".to_string(),
    ];
    let ensured = ensure(&OsProcessRunner, &fx.project, &fx.vault, REPO, KEY, &copy).unwrap();
    assert!(ensured
        .workspace
        .path
        .join(".claude/settings.local.json")
        .is_file());
    assert!(!ensured.workspace.path.join(".env").exists());
}

#[test]
fn it_modified_worktree_is_salvaged_then_removed() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let ensured = ensure(&OsProcessRunner, &fx.project, &fx.vault, REPO, KEY, &[]).unwrap();
    let path = ensured.workspace.path;
    std::fs::write(path.join("a.txt"), "changed").unwrap();
    std::fs::write(path.join("new.txt"), "new").unwrap();
    assert_eq!(modified_count(&OsProcessRunner, &path), 2);
    let removed = remove(&OsProcessRunner, &no_sleep, &fx.project, &path, KEY).unwrap();
    let reference = removed.salvaged_ref.unwrap();
    assert!(reference.starts_with(&format!("{WIP_REF_PREFIX}{KEY}-")));
    assert_eq!(
        reference.len(),
        WIP_REF_PREFIX.len() + KEY.len() + 1 + SHA_PREFIX_LEN
    );
    run_git(&fx.project, &["rev-parse", "--verify", &reference]);
    assert!(!path.exists());
    assert!(run_git(&fx.project, &["branch", "--list", "feat/LOC-1"]).is_empty());
}

#[test]
fn it_clean_worktree_is_removed_without_a_wip_ref() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let ensured = ensure(&OsProcessRunner, &fx.project, &fx.vault, REPO, KEY, &[]).unwrap();
    let removed = remove(
        &OsProcessRunner,
        &no_sleep,
        &fx.project,
        &ensured.workspace.path,
        KEY,
    )
    .unwrap();
    assert!(removed.salvaged_ref.is_none());
    assert!(run_git(&fx.project, &["for-each-ref", WIP_REF_PREFIX]).is_empty());
}

#[test]
fn it_nested_isolation_worktree_works_from_a_linked_worktree() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let ensured = ensure(&OsProcessRunner, &fx.project, &fx.vault, REPO, KEY, &[]).unwrap();
    let path = ensured.workspace.path;
    let nested = path.join(".claude/worktrees/x");
    std::fs::create_dir_all(path.join(".claude/worktrees")).unwrap();
    run_git(
        &path,
        &["worktree", "add", "-b", "iso-x", nested.to_str().unwrap()],
    );
    assert!(nested.join("a.txt").is_file());
    run_git(
        &path,
        &["worktree", "remove", "--force", nested.to_str().unwrap()],
    );
}

#[test]
fn it_scratch_branches_from_freshly_fetched_origin_default() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let origin = fx._root.path().join("origin.git");
    let other = fx._root.path().join("other");
    run_git(
        fx._root.path(),
        &["clone", origin.to_str().unwrap(), "other"],
    );
    run_git(&other, &["config", "user.email", "t@example.com"]);
    run_git(&other, &["config", "user.name", "t"]);
    run_git(&other, &["config", "commit.gpgsign", "false"]);
    std::fs::write(other.join("b.txt"), "b").unwrap();
    run_git(&other, &["add", "."]);
    run_git(&other, &["commit", "-m", "advance"]);
    run_git(&other, &["push", "origin", "master"]);
    let ensured = ensure_scratch(&OsProcessRunner, &fx.project, &fx.vault, REPO, SCRATCH).unwrap();
    assert!(!ensured.reused);
    assert!(ensured.workspace.path.join("b.txt").exists());
    assert_eq!(ensured.workspace.branch.as_deref(), Some("scratch/idea"));
    let again = ensure_scratch(&OsProcessRunner, &fx.project, &fx.vault, REPO, SCRATCH).unwrap();
    assert!(again.reused);
}

#[test]
fn it_releasing_a_dirty_scratch_saves_a_wip_ref_then_removes_it() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let workspace = ensure_scratch(&OsProcessRunner, &fx.project, &fx.vault, REPO, SCRATCH)
        .unwrap()
        .workspace;
    std::fs::write(workspace.path.join("new.txt"), "new").unwrap();
    let removed = release_scratch(&OsProcessRunner, &no_sleep, &fx.project, &workspace).unwrap();
    let reference = removed.salvaged_ref.unwrap();
    assert!(reference.starts_with(&format!("{WIP_REF_PREFIX}scratch-{SCRATCH}-")));
    run_git(&fx.project, &["rev-parse", "--verify", &reference]);
    assert!(!workspace.path.exists());
    assert!(run_git(&fx.project, &["branch", "--list", "scratch/idea"]).is_empty());
}

#[test]
fn it_releasing_a_clean_scratch_removes_it_directly() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let workspace = ensure_scratch(&OsProcessRunner, &fx.project, &fx.vault, REPO, SCRATCH)
        .unwrap()
        .workspace;
    let removed = release_scratch(&OsProcessRunner, &no_sleep, &fx.project, &workspace).unwrap();
    assert!(removed.salvaged_ref.is_none());
    assert!(!workspace.path.exists());
    assert!(run_git(&fx.project, &["for-each-ref", WIP_REF_PREFIX]).is_empty());
}
