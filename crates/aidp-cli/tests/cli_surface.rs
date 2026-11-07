use crate::common;
use common::{aidp, normalize_binary_name, workspace, SUBCOMMANDS};

/// Normalize binary name in CLI output for platform-independent snapshots.
///
/// Delegates to `common::normalize_binary_name`, the single shared
/// implementation also used by `cli_hermetic.rs` via `common::normalize_output`.
fn normalize_for_snapshot(raw: &str) -> String {
    normalize_binary_name(raw)
}

#[test]
fn root_help() {
    let ws = workspace();
    let mut cmd = aidp(&ws);
    let output = cmd.arg("--help").output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = normalize_for_snapshot(&stdout);

    assert!(normalized.contains("ai-dev-pipeline v7 helper"));
    assert!(!normalized.contains("ai-dev-pipeline v4 helper"));

    insta::assert_snapshot!("root_help", normalized);
}

#[test]
fn subcommand_help() {
    let ws = workspace();

    // Wrap in allow_duplicates to silence insta warnings if any snapshot names
    // happen to be duplicated (unlikely, but defensive programming).
    insta::allow_duplicates! {
        for cmd_name in SUBCOMMANDS {
            let mut cmd = aidp(&ws);
            let output = cmd.arg(cmd_name).arg("--help").output().unwrap();

            assert_eq!(output.status.code(), Some(0), "subcommand {} --help failed", cmd_name);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let normalized = normalize_for_snapshot(&stdout);

            let snapshot_name = format!("{}_help", cmd_name);
            insta::assert_snapshot!(snapshot_name, normalized);
        }
    }
}

#[test]
fn version_is_frozen() {
    let ws = workspace();
    let mut cmd = aidp(&ws);
    let output = cmd.arg("--version").output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_ne!(stdout.trim(), "aidp 4.0.0");
    assert!(stdout.trim().starts_with("aidp "));
}

#[test]
fn unknown_subcommand_exits_7() {
    let ws = workspace();
    let mut cmd = aidp(&ws);
    let output = cmd.arg("nosuchcmd").output().unwrap();

    assert_eq!(output.status.code(), Some(7));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout should be empty for error case");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // FIGE-A-DESSEIN: clap error handling; these checks verify the standard
    // error format for unrecognized subcommands.
    assert!(
        stderr.contains("unrecognized subcommand 'nosuchcmd'"),
        "stderr must contain the unrecognized subcommand message"
    );
    assert!(
        stderr.contains("Usage:"),
        "stderr must contain a Usage: line"
    );
    assert!(
        stderr.contains("For more information, try '--help'."),
        "stderr must suggest --help"
    );
}

#[test]
fn ticket_help_frozen_intra_doc_links() {
    // FIGE-A-DESSEIN: aidp ticket --help renders raw rustdoc intra-doc links verbatim.
    // The snapshot must contain the substrings [`Op::Start`] and [`crate::run`]
    // from rust/crates/aidp-tickets/src/cli.rs:222/261/263/280/282.
    let ws = workspace();
    let mut cmd = aidp(&ws);
    let output = cmd.arg("ticket").arg("--help").output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = normalize_for_snapshot(&stdout);

    // Cheap assertion to prove intra-doc links are not rendered
    assert!(normalized.contains("[`Op::Start`]"));
    assert!(normalized.contains("[`crate::run`]"));
}

#[test]
fn status_help_mentions_the_vault() {
    let ws = workspace();
    let mut cmd = aidp(&ws);
    let output = cmd.arg("status").arg("--help").output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = normalize_for_snapshot(&stdout);

    assert!(normalized.contains("List runs found in the vault"));
}

#[test]
fn pipeline_cmd_names_its_three_verbs() {
    let ws = workspace();
    let mut cmd = aidp(&ws);
    let output = cmd.arg("pipeline").arg("--help").output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = normalize_for_snapshot(&stdout);

    for verb in ["list", "show", "expand"] {
        assert!(
            normalized.contains(&format!("  {verb} ")),
            "missing verb {verb}"
        );
    }
}
