//! Git-backed per-task write attribution: flagging, restoring and dirty-before handling.

use aidp_core::process_runner::OsProcessRunner;
use aidp_gateway::native_runs::scope_guard::{
    capture_task_baseline, changed_since, overlapping_writes, restore_task_violations,
    verify_task_writes,
};
use std::path::Path;
use std::process::Command;

const SRC_FILE: &str = "src/a.rs";
const DOCS_FILE: &str = "docs/x.md";
const NEW_DOCS_FILE: &str = "docs/new.md";
const SHARED_FILE: &str = "Cargo.lock";
const SHARED_FILE_SPELLED: &str = "./cargo.lock";
const SIBLING_FILE: &str = "src/b.rs";
const UPPER_SRC_FILE: &str = "SRC/A.rs";
const ORIGINAL_DOCS: &str = "original\n";
const TAMPERED_DOCS: &str = "tampered\n";
const DIRTY_DOCS: &str = "preexisting edit\n";
const SECOND_EDIT_DOCS: &str = "second edit\n";
const SRC_EDIT_ONE: &str = "fn a() { 1; }\n";
const SRC_EDIT_THREE: &str = "fn a() { 3; }\n";
const SIBLING_WORK: &str = "sibling work\n";
const STRAY_CONTENT: &str = "stray\n";
const INITIAL_SRC: &str = "fn a() {}\n";
const CRLF: &str = "\r\n";
const LF: &str = "\n";

fn git_in(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

fn write(dir: &Path, relative: &str, content: &str) {
    let target = dir.join(relative);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, content).unwrap();
}

fn read(dir: &Path, relative: &str) -> String {
    std::fs::read_to_string(dir.join(relative)).unwrap()
}

fn repo_with_docs() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_in(dir.path(), &["init", "-q"]);
    write(dir.path(), SRC_FILE, INITIAL_SRC);
    write(dir.path(), DOCS_FILE, ORIGINAL_DOCS);
    git_in(dir.path(), &["add", "."]);
    git_in(dir.path(), &["commit", "-q", "-m", "init"]);
    dir
}

fn owned(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| path.to_string()).collect()
}

/// A task writing outside its files is flagged and only that write is undone.
#[test]
fn task_write_outside_its_files_is_flagged_and_restored() {
    let repo = repo_with_docs();
    let workspace = repo.path();
    let baseline = capture_task_baseline(workspace, &OsProcessRunner, None);
    write(workspace, SRC_FILE, SRC_EDIT_ONE);
    write(workspace, DOCS_FILE, TAMPERED_DOCS);

    let changed = changed_since(workspace, &baseline, &OsProcessRunner);
    let verdict = verify_task_writes(&changed, &owned(&[SRC_FILE]), &[], &[]);
    restore_task_violations(workspace, &verdict.violations, &baseline, &OsProcessRunner).unwrap();

    assert_eq!(verdict.violations, vec![DOCS_FILE.to_string()]);
    assert_eq!(read(workspace, DOCS_FILE).replace(CRLF, LF), ORIGINAL_DOCS);
    assert_eq!(read(workspace, SRC_FILE), SRC_EDIT_ONE);
}

/// Restoring a stray file leaves a sibling's own file alone.
#[test]
fn sibling_file_is_untouched_by_restore() {
    let repo = repo_with_docs();
    let workspace = repo.path();
    let baseline = capture_task_baseline(workspace, &OsProcessRunner, None);
    write(workspace, SIBLING_FILE, SIBLING_WORK);
    write(workspace, NEW_DOCS_FILE, STRAY_CONTENT);

    let changed = changed_since(workspace, &baseline, &OsProcessRunner);
    let verdict = verify_task_writes(&changed, &owned(&[SRC_FILE]), &[], &[]);
    let own_only: Vec<String> = verdict
        .violations
        .iter()
        .filter(|path| path.as_str() == NEW_DOCS_FILE)
        .cloned()
        .collect();
    restore_task_violations(workspace, &own_only, &baseline, &OsProcessRunner).unwrap();

    assert!(!workspace.join(NEW_DOCS_FILE).exists());
    assert_eq!(read(workspace, SIBLING_FILE), SIBLING_WORK);
}

/// A shared file is a violation even when the task lists it.
#[test]
fn shared_file_write_is_flagged_even_when_listed_for_the_task() {
    let changed = owned(&[SHARED_FILE]);
    let verdict = verify_task_writes(
        &changed,
        &owned(&[SHARED_FILE]),
        &[],
        &owned(&[SHARED_FILE_SPELLED]),
    );
    assert_eq!(verdict.violations, changed);
}

/// An allowed extra path is not a violation.
#[test]
fn allowed_extra_is_not_a_violation() {
    let changed = owned(&[DOCS_FILE]);
    let verdict = verify_task_writes(&changed, &owned(&[SRC_FILE]), &owned(&[DOCS_FILE]), &[]);
    assert!(verdict.is_clean());
}

/// A file dirty before the task and left alone is not attributed to it.
#[test]
fn file_dirty_before_and_unchanged_is_not_attributed() {
    let repo = repo_with_docs();
    let workspace = repo.path();
    write(workspace, DOCS_FILE, DIRTY_DOCS);
    let baseline = capture_task_baseline(workspace, &OsProcessRunner, None);
    write(workspace, SRC_FILE, SRC_EDIT_THREE);

    let changed = changed_since(workspace, &baseline, &OsProcessRunner);

    assert_eq!(changed, vec![SRC_FILE.to_string()]);
}

/// A dirty file edited again during the task is attributed to it.
#[test]
fn dirty_file_edited_again_is_attributed() {
    let repo = repo_with_docs();
    let workspace = repo.path();
    write(workspace, DOCS_FILE, DIRTY_DOCS);
    let baseline = capture_task_baseline(workspace, &OsProcessRunner, None);
    write(workspace, DOCS_FILE, SECOND_EDIT_DOCS);

    assert_eq!(
        changed_since(workspace, &baseline, &OsProcessRunner),
        vec![DOCS_FILE.to_string()]
    );
}

/// An out-of-scope write to a file dirty before the task gets the pre-task content back.
#[test]
fn dirty_before_file_written_out_of_scope_is_restored_to_its_pre_task_content() {
    let repo = repo_with_docs();
    let workspace = repo.path();
    write(workspace, DOCS_FILE, DIRTY_DOCS);
    let baseline = capture_task_baseline(workspace, &OsProcessRunner, None);
    write(workspace, DOCS_FILE, TAMPERED_DOCS);

    let changed = changed_since(workspace, &baseline, &OsProcessRunner);
    let verdict = verify_task_writes(&changed, &owned(&[SRC_FILE]), &[], &[]);
    restore_task_violations(workspace, &verdict.violations, &baseline, &OsProcessRunner).unwrap();

    assert_eq!(verdict.violations, vec![DOCS_FILE.to_string()]);
    assert_eq!(read(workspace, DOCS_FILE), DIRTY_DOCS);
}

/// Paths changed by both tasks are reported once per side, spelled as in the first list.
#[test]
fn overlapping_writes_returns_paths_changed_by_both_tasks() {
    let first = owned(&[SRC_FILE, DOCS_FILE]);
    let second = owned(&[UPPER_SRC_FILE, SIBLING_FILE]);

    assert_eq!(
        overlapping_writes(&first, &second),
        vec![SRC_FILE.to_string()]
    );
    assert_eq!(
        overlapping_writes(&second, &first),
        vec![UPPER_SRC_FILE.to_string()]
    );
    assert!(overlapping_writes(&first, &[]).is_empty());
}
