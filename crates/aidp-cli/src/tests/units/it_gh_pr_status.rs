//! `aidp gh pr-status` is a registered verb with its documented flags.

#[path = "../../../tests/common/mod.rs"]
mod common;

use aidp_testkit::ProcShim;
use common::*;

const GH: &str = "gh";
const HELP_FLAG: &str = "--help";
const VERB: &str = "pr-status";
const REPO_FLAG: &str = "--repo";
const REPO_SLUG: &str = "owner/name";
const PR_NUMBER: &str = "8";
const GRAPHQL_ARG: &str = "graphql";
const COMPARE_SEGMENT: &str = "compare";
const CLOSED_PAYLOAD: &str = include_str!("fixtures/pr_status/graphql_closed.json");
const CLOSED_STATE: &str = "CLOSED";

/// The help of the verb lists its repository flag.
#[test]
fn pr_status_help_lists_the_verb_and_repo_flag() {
    let ws = workspace();
    let output = aidp(&ws)
        .args([GH, VERB, HELP_FLAG])
        .output()
        .expect("run aidp gh pr-status --help");
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains(REPO_FLAG), "help should list --repo: {help}");
}

/// The help of `gh` lists the verb.
#[test]
fn gh_help_lists_pr_status() {
    let ws = workspace();
    let output = aidp(&ws)
        .args([GH, HELP_FLAG])
        .output()
        .expect("run aidp gh --help");
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains(VERB));
}

/// The closed-PR fixture really carries the closed state.
#[test]
fn closed_fixture_carries_the_closed_state() {
    assert!(CLOSED_PAYLOAD.contains(CLOSED_STATE));
}

/// With a fake `gh` on PATH, the status read stops after the GraphQL call:
/// the last recorded invocation is never a compare request.
#[test]
fn pr_status_never_calls_compare_through_the_gh_shim() {
    let ws = workspace();
    let shim = ProcShim::new(GH, &ws);
    let _ = aidp(&ws)
        .args([GH, VERB, PR_NUMBER, REPO_FLAG, REPO_SLUG])
        .output()
        .expect("run aidp gh pr-status");
    let argv = shim.argv();
    assert!(
        argv.iter().any(|arg| arg == GRAPHQL_ARG),
        "gh should have been asked for graphql: {argv:?}"
    );
    assert!(
        !argv.iter().any(|arg| arg.contains(COMPARE_SEGMENT)),
        "no compare call expected: {argv:?}"
    );
}
