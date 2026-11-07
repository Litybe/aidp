use crate::common;
use common::*;

const NOT_FOUND_EXIT: i32 = 3;
const USAGE_EXIT: i32 = 7;

#[test]
fn help_lists_the_exec_flags() {
    let ws = workspace();
    let out = aidp(&ws)
        .args(["run", "exec", "--help"])
        .output()
        .expect("help");
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--target",
        "--inputs-file",
        "--input",
        "--provider",
        "--approve-gate",
        "--foreground",
    ] {
        assert!(help.contains(flag), "missing {flag} in {help}");
    }
}

#[test]
fn unknown_template_exits_not_found() {
    let ws = workspace();
    let out = aidp_with_fixture_skill(&ws)
        .args([
            "run",
            "exec",
            "no-such-template",
            "--target",
            "X-1",
            "--no-vault",
        ])
        .output()
        .expect("exec");
    assert_eq!(out.status.code(), Some(NOT_FOUND_EXIT));
}

#[test]
fn malformed_input_exits_usage() {
    let ws = workspace();
    let out = aidp_with_fixture_skill(&ws)
        .args([
            "run",
            "exec",
            "dev",
            "--target",
            "X-1",
            "--input",
            "nokeyvalue",
        ])
        .output()
        .expect("exec");
    assert_eq!(out.status.code(), Some(USAGE_EXIT));
}
