use super::{run_vault, seeded_inputs};
use crate::models::constants::flow_inputs::INPUT_PROMPT;
use crate::models::native_run_start::StartNativeRun;
use aidp_core::run::{INPUT_JIRA_KEY, INPUT_TARGET};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const KEY: &str = "LOC-7";
const OTHER_KEY: &str = "LOC-8";
const PROMPT: &str = "Add a retry button";
const BLANK_PROMPT: &str = "  ";
const INSTANCE_VAULT: &str = "instance-vault";
const REQUEST_VAULT: &str = "request-vault";

#[test]
fn configured_native_vault_is_used_and_empty_host_vault_disables_global_fallback() {
    let workspace = aidp_testkit::TempWorkspace::new(INSTANCE_VAULT);
    let instance = workspace.child(INSTANCE_VAULT);
    let explicit = workspace.child(REQUEST_VAULT);
    std::fs::create_dir_all(&instance).unwrap();
    std::fs::create_dir_all(&explicit).unwrap();
    let request = StartNativeRun::default();
    assert_eq!(
        run_vault(&request, workspace.path(), Some(&instance))
            .unwrap()
            .root,
        instance
    );
    assert!(run_vault(&request, workspace.path(), Some(std::path::Path::new(""))).is_none());
    let request = StartNativeRun {
        vault: Some(explicit.clone()),
        ..Default::default()
    };
    assert_eq!(
        run_vault(&request, workspace.path(), Some(&instance))
            .unwrap()
            .root,
        explicit
    );
}

fn request(inputs: BTreeMap<String, Value>) -> StartNativeRun {
    StartNativeRun {
        target: Some(KEY.to_string()),
        inputs,
        ..StartNativeRun::default()
    }
}

#[test]
fn a_target_seeds_target_and_jira_key() {
    let inputs = seeded_inputs(&request(BTreeMap::new()));
    assert_eq!(inputs[INPUT_TARGET], json!(KEY));
    assert_eq!(inputs[INPUT_JIRA_KEY], json!(KEY));
}

#[test]
fn a_prompt_driven_run_keeps_the_target_but_gets_no_jira_key() {
    let inputs = seeded_inputs(&request(BTreeMap::from([(
        INPUT_PROMPT.to_string(),
        json!(PROMPT),
    )])));
    assert_eq!(inputs[INPUT_TARGET], json!(KEY));
    assert!(!inputs.contains_key(INPUT_JIRA_KEY));
}

#[test]
fn a_blank_prompt_does_not_make_the_run_prompt_driven() {
    let inputs = seeded_inputs(&request(BTreeMap::from([(
        INPUT_PROMPT.to_string(),
        json!(BLANK_PROMPT),
    )])));
    assert_eq!(inputs[INPUT_JIRA_KEY], json!(KEY));
}

#[test]
fn explicit_inputs_win_over_the_seeds() {
    let inputs = seeded_inputs(&request(BTreeMap::from([
        (INPUT_TARGET.to_string(), json!(OTHER_KEY)),
        (INPUT_JIRA_KEY.to_string(), json!(OTHER_KEY)),
    ])));
    assert_eq!(inputs[INPUT_TARGET], json!(OTHER_KEY));
    assert_eq!(inputs[INPUT_JIRA_KEY], json!(OTHER_KEY));
}
