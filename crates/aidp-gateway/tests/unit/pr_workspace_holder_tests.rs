use super::*;
use crate::models::constants::pr_workspace_holder::HOLDER_FILE_NAME;
use crate::models::constants::pr_workspace_holder::{INPUT_PR, INPUT_PR_NUMBER, INPUT_TARGET};
use aidp_testkit::TempWorkspace;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const REPO_SEGMENT: &str = "svc";
const NUMBER: u64 = 7;
const STAGE: &str = "implement";
const RUN_ID: &str = "run-1";
const ACQUIRED_AT: &str = "2026-10-08T10:00:00.000Z";
const PULL_URL: &str = "https://github.com/acme/svc/pull/123";
const PULL_URL_SLASH: &str = "https://github.com/acme/svc/pull/123/";
const TICKET_KEY: &str = "LOC-85";
const BROKEN_CONTENT: &str = "{ not json";

fn holder() -> PrWorkspaceHolder {
    PrWorkspaceHolder {
        stage: STAGE.to_string(),
        run_id: Some(RUN_ID.to_string()),
        session_id: None,
        acquired_at: ACQUIRED_AT.to_string(),
    }
}

/// A written holder reads back identically and sits next to the PR worktree.
#[test]
fn write_then_read_returns_the_same_holder() {
    let workspace = TempWorkspace::new("gw-holder-roundtrip");
    write(workspace.path(), REPO_SEGMENT, NUMBER, &holder()).unwrap();

    assert_eq!(read(workspace.path(), REPO_SEGMENT, NUMBER), Some(holder()));
    assert_eq!(
        holder_path(workspace.path(), REPO_SEGMENT, NUMBER).file_name(),
        Some(std::ffi::OsStr::new(HOLDER_FILE_NAME))
    );
}

/// An absent file reads as no holder.
#[test]
fn read_without_a_file_is_none() {
    let workspace = TempWorkspace::new("gw-holder-absent");

    assert_eq!(read(workspace.path(), REPO_SEGMENT, NUMBER), None);
}

/// An unparseable file reads as no holder instead of failing.
#[test]
fn read_of_a_broken_file_is_none() {
    let workspace = TempWorkspace::new("gw-holder-broken");
    let path = holder_path(workspace.path(), REPO_SEGMENT, NUMBER);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, BROKEN_CONTENT).unwrap();

    assert_eq!(read(workspace.path(), REPO_SEGMENT, NUMBER), None);
}

/// Removing deletes the file and tolerates a second removal.
#[test]
fn remove_deletes_the_file_and_is_idempotent() {
    let workspace = TempWorkspace::new("gw-holder-remove");
    write(workspace.path(), REPO_SEGMENT, NUMBER, &holder()).unwrap();

    remove(workspace.path(), REPO_SEGMENT, NUMBER);
    remove(workspace.path(), REPO_SEGMENT, NUMBER);

    assert!(!holder_path(workspace.path(), REPO_SEGMENT, NUMBER).exists());
}

/// The description names the stage and the run.
#[test]
fn describe_names_the_stage_and_the_run() {
    let text = holder().describe();

    assert!(text.contains(STAGE) && text.contains(RUN_ID), "{text}");
}

fn inputs_of(name: &str, value: Value) -> BTreeMap<String, Value> {
    BTreeMap::from([(name.to_string(), value)])
}

/// Every accepted spelling of a pull request resolves to its number.
#[test]
fn pr_number_of_accepts_number_string_hash_and_url() {
    for value in [
        json!(123),
        json!("123"),
        json!(" #123 "),
        json!(PULL_URL),
        json!(PULL_URL_SLASH),
    ] {
        assert_eq!(
            pr_number_of(&inputs_of(INPUT_PR, value.clone())),
            Some(123),
            "{value}"
        );
    }
}

/// The three input names are all read.
#[test]
fn pr_number_of_reads_pr_number_pr_and_target() {
    for name in [INPUT_PR_NUMBER, INPUT_PR, INPUT_TARGET] {
        assert_eq!(pr_number_of(&inputs_of(name, json!(9))), Some(9), "{name}");
    }
}

/// An unusable `target` such as a ticket key does not hide a later usable input.
#[test]
fn pr_number_of_skips_unparseable_values_and_follows_the_name_order() {
    let mut inputs = inputs_of(INPUT_PR_NUMBER, json!(TICKET_KEY));
    inputs.insert(INPUT_PR.to_string(), json!(5));
    inputs.insert(INPUT_TARGET.to_string(), json!(6));

    assert_eq!(pr_number_of(&inputs), Some(5));
    assert_eq!(
        pr_number_of(&inputs_of(INPUT_TARGET, json!(TICKET_KEY))),
        None
    );
    assert_eq!(pr_number_of(&inputs_of(INPUT_PR, json!(true))), None);
    assert_eq!(pr_number_of(&BTreeMap::new()), None);
}
