use super::*;
use aidp_testkit::TempWorkspace;
use std::time::Duration;

const WORKSPACE_TAG: &str = "aidp-binary";
const RELEASE_BINARY: &str = "/skill/rust/target/release/aidp";
const DEBUG_BINARY: &str = "/skill/rust/target/debug/aidp";
const WORKTREE_BINARY: &str = "/repo/.claude/worktrees/x/rust/target/release/aidp";
const OTHER_BINARY: &str = "/other/bin/aidp";
const EXPLICIT_DIR: &str = "explicit";
const RUNNING_DIR: &str = "running";
const BUILD_DIR: &str = "build";
const EMPTY_DIR: &str = "empty";
const MOVED_DIR: &str = "moved";
const WORKSPACE_MANIFEST: &str = "Cargo.toml";
const WORKSPACE_CRATES: &str = "crates";
const RELEASE_EXE: &str = "target/release/aidp";
const INSTALLED_EXE: &str = ".aidp/bin/aidp";

fn at(seconds: u64) -> Option<SystemTime> {
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
}

fn facts(resolved: &str, running: &str) -> AidpBinaryFacts {
    AidpBinaryFacts {
        resolved: Some(PathBuf::from(resolved)),
        running: Some(PathBuf::from(running)),
        ..AidpBinaryFacts::default()
    }
}

fn issues_of(facts: AidpBinaryFacts) -> Vec<AidpBinaryIssue> {
    evaluate(facts).issues
}

fn write_file_at(path: &Path, seconds: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap();
}

#[test]
fn absent_when_nothing_resolved() {
    let report = evaluate(AidpBinaryFacts::default());
    assert_eq!(report.issues, vec![AidpBinaryIssue::Absent]);
    assert!(!report.ok());
}

#[test]
fn identical_paths_are_ok() {
    let report = evaluate(facts(RELEASE_BINARY, RELEASE_BINARY));
    assert!(report.ok());
}

#[test]
fn different_paths_are_a_mismatch() {
    assert_eq!(
        issues_of(facts(OTHER_BINARY, RELEASE_BINARY)),
        vec![AidpBinaryIssue::Mismatch]
    );
}

#[test]
fn older_than_running_requires_a_mismatch() {
    let mut same = facts(RELEASE_BINARY, RELEASE_BINARY);
    same.resolved_modified = at(1);
    same.running_modified = at(2);
    assert!(issues_of(same).is_empty());

    let mut different = facts(OTHER_BINARY, RELEASE_BINARY);
    different.resolved_modified = at(1);
    different.running_modified = at(2);
    assert_eq!(
        issues_of(different),
        vec![AidpBinaryIssue::Mismatch, AidpBinaryIssue::OlderThanRunning]
    );
}

#[test]
fn newer_path_binary_with_mismatch_reports_mismatch_only() {
    let mut newer = facts(OTHER_BINARY, RELEASE_BINARY);
    newer.resolved_modified = at(5);
    newer.running_modified = at(2);
    assert_eq!(issues_of(newer), vec![AidpBinaryIssue::Mismatch]);
}

#[test]
fn canonical_release_install_is_not_a_target_build() {
    assert!(issues_of(facts(RELEASE_BINARY, RELEASE_BINARY)).is_empty());
}

#[test]
fn debug_build_is_a_target_build() {
    assert_eq!(
        issues_of(facts(DEBUG_BINARY, DEBUG_BINARY)),
        vec![AidpBinaryIssue::TargetBuild]
    );
}

#[test]
fn worktree_build_is_a_target_build() {
    assert_eq!(
        issues_of(facts(WORKTREE_BINARY, WORKTREE_BINARY)),
        vec![AidpBinaryIssue::TargetBuild]
    );
}

#[test]
fn older_than_sources_is_reported() {
    let mut stale = facts(RELEASE_BINARY, RELEASE_BINARY);
    stale.resolved_modified = at(1);
    stale.sources_modified = at(9);
    assert_eq!(issues_of(stale), vec![AidpBinaryIssue::OlderThanSources]);
}

#[test]
fn unknown_times_produce_no_time_issue() {
    let mut unknown = facts(OTHER_BINARY, RELEASE_BINARY);
    unknown.resolved_modified = at(1);
    assert_eq!(issues_of(unknown), vec![AidpBinaryIssue::Mismatch]);

    let mut no_resolved_time = facts(RELEASE_BINARY, RELEASE_BINARY);
    no_resolved_time.sources_modified = at(9);
    assert!(issues_of(no_resolved_time).is_empty());
}

#[cfg(windows)]
#[test]
fn verbatim_prefix_and_case_are_normalized() {
    let report = evaluate(facts(
        r"\\?\C:\Skill\rust\target\release\aidp.exe",
        r"c:\skill\rust\target\release\AIDP.exe",
    ));
    assert!(report.ok());
}

#[test]
fn normalize_unifies_separators() {
    assert_eq!(
        normalize(Path::new(r"\\?\UNC\host\share\aidp")),
        normalize(Path::new(r"\\host\share\aidp"))
    );
}

#[test]
fn to_json_exposes_the_camel_case_keys() {
    let mut stale = facts(OTHER_BINARY, RELEASE_BINARY);
    stale.resolved_modified = at(0);
    let json = evaluate(stale).to_json();
    let object = json.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "issues",
            "modifiedAt",
            "ok",
            "path",
            "runningModifiedAt",
            "runningPath"
        ]
    );
    assert_eq!(object["ok"], false);
    assert_eq!(object["issues"], serde_json::json!([ISSUE_MISMATCH]));
    assert_eq!(object["modifiedAt"], "1970-01-01T00:00:00+00:00");
    assert!(object["runningModifiedAt"].is_null());
}

#[test]
fn issue_slugs_are_stable() {
    assert_eq!(AidpBinaryIssue::Absent.slug(), "absent");
    assert_eq!(AidpBinaryIssue::TargetBuild.slug(), "target_build");
    assert_eq!(AidpBinaryIssue::Mismatch.slug(), "mismatch");
    assert_eq!(
        AidpBinaryIssue::OlderThanRunning.slug(),
        "older_than_running"
    );
    assert_eq!(
        AidpBinaryIssue::OlderThanSources.slug(),
        "older_than_sources"
    );
}

#[test]
fn newest_source_mtime_picks_newest_and_ignores_target() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let root = workspace.path();
    write_file_at(&root.join("Cargo.toml"), 10);
    write_file_at(&root.join("crates/a/src/lib.rs"), 20);
    write_file_at(&root.join("crates/a/src/deep/mod.rs"), 40);
    write_file_at(&root.join("crates/a/src/notes.txt"), 90);
    write_file_at(&root.join("crates/a/src/target/generated.rs"), 99);
    assert_eq!(newest_source_mtime(root), at(40));
}

#[test]
fn newest_source_mtime_is_none_without_sources() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    assert_eq!(newest_source_mtime(workspace.path()), None);
}

#[test]
fn rust_dir_is_the_ancestor_holding_cargo_toml_and_crates() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let rust_dir = workspace.path().join("rust");
    write_file_at(&rust_dir.join("Cargo.toml"), 1);
    std::fs::create_dir_all(rust_dir.join("crates")).unwrap();
    let running = rust_dir.join("target/release/aidp");
    assert_eq!(rust_dir_from_ancestors(&running), Some(rust_dir));
}

fn fake_workspace(dir: &Path) {
    write_file_at(&dir.join(WORKSPACE_MANIFEST), 1);
    std::fs::create_dir_all(dir.join(WORKSPACE_CRATES)).unwrap();
}

#[test]
fn an_explicit_sources_dir_wins_over_the_running_and_build_workspaces() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let explicit = workspace.path().join(EXPLICIT_DIR);
    let running_dir = workspace.path().join(RUNNING_DIR);
    let build = workspace.path().join(BUILD_DIR);
    for dir in [&explicit, &running_dir, &build] {
        fake_workspace(dir);
    }
    let running = running_dir.join(RELEASE_EXE);
    assert_eq!(
        rust_dir_from(Some(explicit.clone()), &running, &build),
        Some(explicit)
    );
    let not_a_workspace = workspace.path().join(EMPTY_DIR);
    assert_eq!(
        rust_dir_from(Some(not_a_workspace), &running, &build),
        Some(running_dir)
    );
}

#[test]
fn an_installed_binary_falls_back_to_the_workspace_it_was_built_in() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let build = workspace.path().join(BUILD_DIR);
    fake_workspace(&build);
    let installed = workspace.path().join(INSTALLED_EXE);
    assert_eq!(rust_dir_from(None, &installed, &build), Some(build.clone()));
    let moved = workspace.path().join(MOVED_DIR);
    assert_eq!(rust_dir_from(None, &installed, &moved), None);
}

#[test]
fn the_build_workspace_is_this_repository() {
    assert!(is_workspace(&build_workspace()));
}

#[test]
fn canonical_or_self_falls_back_to_the_input() {
    let missing = Path::new("/definitely/not/here/aidp");
    assert_eq!(canonical_or_self(missing), missing);
    assert_eq!(modified(missing), None);
}
