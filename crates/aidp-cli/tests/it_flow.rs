//! End-to-end tests for the local verbs of `aidp flow`.

use crate::common;
use aidp_core::ExitStatus;
use common::*;
use std::path::PathBuf;

const FLOW_ARG: &str = "flow";
const VALIDATE_ARG: &str = "validate";
const SHOW_ARG: &str = "show";
const SKILL_ROOT_VAR: &str = "AIDP_SKILL_ROOT";
const VALID_FIXTURE: &str = "valid.yaml";
const UNKNOWN_USE_CODE: &str = "unknown_use";
const DUPLICATE_CODE: &str = "duplicate_step_id";
const UNKNOWN_STEP_CODE: &str = "unknown_step_ref";
const UNBOUNDED_CODE: &str = "loop_unbounded";
const UNKNOWN_PIPELINE_CODE: &str = "unknown_pipeline_template";
const UNKNOWN_PIPELINE_FIXTURE: &str = "unknown_pipeline.yaml";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("flows")
        .join(name)
}

fn skill_root() -> PathBuf {
    aidp_testkit::fixtures::skill_root()
}

fn run_flow(verb: &str, path: PathBuf) -> std::process::Output {
    let ws = workspace();
    let mut cmd = aidp(&ws);
    cmd.env(SKILL_ROOT_VAR, skill_root())
        .arg(FLOW_ARG)
        .arg(verb)
        .arg(path);
    cmd.output().expect("failed to run aidp flow")
}

fn assert_rejected_with(fixture_name: &str, code: &str) {
    let output = run_flow(VALIDATE_ARG, fixture(fixture_name));
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::NegativeVerdict.code() as i32),
        "{fixture_name} must exit with a negative verdict"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(code), "stdout lacks {code}: {stdout}");
}

#[test]
fn validate_accepts_a_valid_flow() {
    let output = run_flow(VALIDATE_ARG, fixture(VALID_FIXTURE));
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::Success.code() as i32),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"ok\":true"));
}

#[test]
fn validate_rejects_duplicate_ids() {
    assert_rejected_with("duplicate_ids.yaml", DUPLICATE_CODE);
}

#[test]
fn validate_rejects_unknown_use() {
    assert_rejected_with("unknown_use.yaml", UNKNOWN_USE_CODE);
}

#[test]
fn validate_rejects_when_on_unknown_step() {
    assert_rejected_with("unknown_when.yaml", UNKNOWN_STEP_CODE);
}

#[test]
fn validate_rejects_loop_without_bound() {
    assert_rejected_with("unbounded_loop.yaml", UNBOUNDED_CODE);
}

#[test]
fn validate_missing_file_exits_not_found() {
    let output = run_flow(VALIDATE_ARG, fixture("absent.yaml"));
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::NotFound.code() as i32)
    );
}

#[test]
fn show_prints_the_definition() {
    let output = run_flow(SHOW_ARG, fixture(VALID_FIXTURE));
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::Success.code() as i32)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"id\":\"test-flow\""), "stdout: {stdout}");
    assert!(stdout.contains("develop"));
}

#[test]
fn daemon_verb_with_unreachable_daemon_exits_upstream_failure() {
    let ws = workspace();
    let mut cmd = aidp(&ws);
    cmd.arg(FLOW_ARG).arg("list");
    let output = cmd.output().expect("failed to run aidp flow list");
    assert_eq!(
        output.status.code(),
        Some(ExitStatus::UpstreamFailure.code() as i32)
    );
}

/// A pipeline step naming a template absent from the catalog is rejected.
#[test]
fn validate_rejects_unknown_pipeline_template() {
    assert_rejected_with(UNKNOWN_PIPELINE_FIXTURE, UNKNOWN_PIPELINE_CODE);
}
