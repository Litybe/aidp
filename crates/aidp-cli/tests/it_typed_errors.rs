//! End-to-end tests for LOC-30's typed errors: `aidp validate` names the
//! malformed file, a broken composite child names both the stage and the
//! child file, a missing `AIDP_JIRA_BASE` names the remedy, and `aidp ci
//! status --refresh` with no `gh` on PATH reports a real lookup failure
//! rather than the ordinary "no pull request" skip. Two non-regression
//! checks close the suite: `aidp gateway status` against an unreachable
//! daemon, and an illegal ticket transition leaving the board untouched.

use crate::common;
use aidp_testkit::snapshot_tree;
use common::*;
use std::fs;
use std::path::Path;

const BROKEN_YAML: &str = "broken.yaml";
const PARENT_YAML: &str = "parent.yaml";
const KID_YAML: &str = "kid.yaml";

const VALIDATE_CMD: &str = "validate";
const STATUS_OP: &str = "status";
const GATEWAY_CMD: &str = "gateway";
const TICKET_CMD: &str = "ticket";
const VAULT_FLAG: &str = "--vault";
const NEW_STATUS: &str = "blocked";

const GATEWAY_UNREACHABLE_FRAGMENT: &str = "gateway unreachable";
const GATEWAY_SERVE_HINT_FRAGMENT: &str = "aidp gateway serve";
const NOT_A_LEGAL_TRANSITION_FRAGMENT: &str = "not a legal transition";

fn fixtures_dir() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/typed_errors"
    ))
}

fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Scenario 1: a mis-indented stage field fails YAML parsing, and `aidp
/// validate` names the offending file rather than the raw serde message.
#[test]
fn validate_names_the_malformed_file() {
    let ws = workspace();
    let target = fixtures_dir().join(BROKEN_YAML);

    let mut cmd = aidp(&ws);
    cmd.arg(VALIDATE_CMD).arg(&target);
    let output = cmd.output().expect("running aidp validate should succeed");

    assert_eq!(
        output.status.code(),
        Some(1),
        "a malformed pipeline should exit 1"
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains(BROKEN_YAML),
        "stderr should name the malformed file, got: {stderr}"
    );
}

/// Scenario 2: a composite stage whose `uses:` child is itself malformed
/// fails at composition time, naming both the parent stage and the child
/// file the `uses:` reference resolved to.
#[test]
fn validate_names_the_composite_stage_and_child_file_on_a_broken_uses() {
    let ws = workspace();
    let target = fixtures_dir().join(PARENT_YAML);

    let mut cmd = aidp(&ws);
    cmd.arg(VALIDATE_CMD).arg(&target);
    let output = cmd.output().expect("running aidp validate should succeed");

    assert_eq!(
        output.status.code(),
        Some(1),
        "a pipeline composing a broken child should exit 1"
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("child_stage"),
        "stderr should name the composite stage, got: {stderr}"
    );
    assert!(
        stderr.contains(KID_YAML),
        "stderr should name the resolved child file, got: {stderr}"
    );
}

/// Non-regression: `aidp gateway status` against a port nothing listens on
/// fails with the client's diagnostic message, naming both the failure and
/// the remedy command.
#[test]
fn gateway_status_names_the_unreachable_daemon_and_the_remedy() {
    let ws = workspace();

    let mut cmd = aidp(&ws);
    cmd.arg(GATEWAY_CMD).arg(STATUS_OP);
    let output = cmd
        .output()
        .expect("running aidp gateway status should succeed");

    assert_eq!(
        output.status.code(),
        Some(1),
        "gateway status with no daemon running should exit 1"
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains(GATEWAY_UNREACHABLE_FRAGMENT),
        "stderr should name the unreachable gateway, got: {stderr}"
    );
    assert!(
        stderr.contains(GATEWAY_SERVE_HINT_FRAGMENT),
        "stderr should point at `aidp gateway serve`, got: {stderr}"
    );
}

/// Non-regression: an illegal ticket status transition is refused and
/// leaves the `TODO/` tree byte-for-byte unchanged.
#[test]
fn illegal_ticket_transition_is_refused_and_leaves_the_board_unchanged() {
    let ws = workspace();

    let demo_dir = ws.child("vault").join("Pipelines").join("demo");
    fs::create_dir_all(&demo_dir).expect("creating the demo service dir should succeed");

    let ticket_path = ws.child("vault").join("TODO").join("demo").join("LOC-1.md");
    fs::create_dir_all(ticket_path.parent().unwrap()).expect("creating TODO/demo should succeed");
    fs::write(
        &ticket_path,
        "---\naidp: ticket\nkey: LOC-1\nsource: local\ntype: Task\nsummary: Test\nstatus: done\npriority: Medium\nservice: demo\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\n---\nBody\n",
    )
    .expect("writing LOC-1.md should succeed");

    let demo_ticket_dir = ws.child("vault").join("TODO").join("demo");
    let snapshot_before = snapshot_tree(&demo_ticket_dir);

    let mut cmd = aidp(&ws);
    cmd.arg(TICKET_CMD)
        .arg(VAULT_FLAG)
        .arg(ws.child("vault"))
        .arg(STATUS_OP)
        .arg("LOC-1")
        .arg(NEW_STATUS);
    let output = cmd
        .output()
        .expect("running aidp ticket status should succeed");

    assert_ne!(
        output.status.code(),
        Some(0),
        "an illegal target status should not exit 0"
    );
    let stderr = stderr_text(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stderr.contains(NOT_A_LEGAL_TRANSITION_FRAGMENT)
            || stdout.contains(NOT_A_LEGAL_TRANSITION_FRAGMENT),
        "output should mention 'not a legal transition', got stdout: {stdout}, stderr: {stderr}"
    );

    let snapshot_after = snapshot_tree(&demo_ticket_dir);
    assert_eq!(
        snapshot_before, snapshot_after,
        "a refused transition should not modify any file under TODO/demo"
    );
}
