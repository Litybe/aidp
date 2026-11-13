//! The fixture repository copies are real, independent repositories.

use std::process::Command;

use aidp_testkit::{GitFixture, TempWorkspace};

const README: &str = "README.md";
const README_CONTENTS: &str = "fixture\n";
const IDENTITY_NAME: &str = "fixture";
const IDENTITY_EMAIL: &str = "fixture@example.test";
const BRANCH: &str = "main";
const FIXTURE: GitFixture<'static> = GitFixture {
    files: &[(README, README_CONTENTS)],
    empty_directories: &[],
    branch: Some(BRANCH),
    user_name: IDENTITY_NAME,
    user_email: IDENTITY_EMAIL,
    autocrlf_off: true,
};

fn git_output(directory: &std::path::Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {arguments:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn a_copy_with_origin_is_clean_and_points_at_its_own_remote() {
    let root = TempWorkspace::new("git-fixture");
    let workspace = root.child("workspace");
    let origin = root.child("origin.git");

    FIXTURE.copy_with_origin_into(&workspace, &origin);

    assert_eq!(git_output(&workspace, &["status", "--porcelain"]), "");
    assert_eq!(
        git_output(&workspace, &["config", "--get", "remote.origin.url"]),
        origin.to_str().unwrap()
    );
    assert_eq!(
        git_output(&workspace, &["rev-parse", "HEAD"]),
        git_output(&origin, &["rev-parse", BRANCH])
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join(README)).unwrap(),
        README_CONTENTS
    );
}

#[test]
fn a_plain_copy_is_a_committed_repository_without_a_remote() {
    let root = TempWorkspace::new("git-fixture-plain");
    let workspace = root.child("workspace");

    FIXTURE.copy_into(&workspace);

    assert_eq!(git_output(&workspace, &["status", "--porcelain"]), "");
    assert_eq!(git_output(&workspace, &["remote"]), "");
    assert_eq!(
        git_output(&workspace, &["log", "--format=%an <%ae>"]),
        format!("{IDENTITY_NAME} <{IDENTITY_EMAIL}>")
    );
}
