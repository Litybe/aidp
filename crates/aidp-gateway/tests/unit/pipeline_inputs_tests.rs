use super::{build, report_path};
use crate::models::constants::flow_inputs::{
    INPUT_FEATURE, INPUT_ISSUE, INPUT_PROMPT, INPUT_PUBLISH, INPUT_TRACK, PROMPT_SEPARATOR,
    PUBLISH_BY_HOST, TICKET_SOURCE, TICKET_SUMMARY, TICKET_TRACK, WITH_REPORT_FROM,
};
use aidp_flows::model::PUBLISH_BLOCK;
use aidp_flows::{FlowDef, FlowState, StepState, OUTPUT_REPORT};
use aidp_tickets::model::Source;
use aidp_tickets::Track;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const KEY: &str = "GH-7";
const TICKET_KEY_FIELD: &str = "key";
const BODY: &str = "  Add a retry button to the run page.\n";
const TRIMMED_BODY: &str = "Add a retry button to the run page.";
const SUMMARY: &str = "Retry button";
const REPORT: &str = "## Bug 1\n\nThe button does nothing.";
const REPORT_PATH: &str = "C:/vault/flow/reports/1.md";
const VERDICT_STEP: &str = "verdict";
const DECLARED_INPUT: &str = "ticket";
const DECLARATION_REQUIRED: &str = "required";
const WITH_TRACK: &str = "deep";
const FLOW_ID: &str = "test";
const OTHER_FEATURE: &str = "other";
const BLANK: &str = "   ";
const EXECUTION_KEY: &str = "TEST-31-run-a";

#[test]
fn sibling_execution_uses_ticket_for_the_issue_and_execution_for_feature() {
    let mut state = state(Source::Github, None);
    state.key = EXECUTION_KEY.to_string();
    let inputs = build(&state, None, None, &BTreeMap::new());
    assert_eq!(inputs[INPUT_ISSUE], json!("7"));
    assert_eq!(inputs[INPUT_FEATURE], json!(EXECUTION_KEY));
}

#[test]
fn external_ticket_description_supplies_demand_without_a_vault_note() {
    let mut state = state(Source::Local, None);
    state.ticket[crate::models::constants::flow_execution::TICKET_DESCRIPTION] = json!(BODY);
    let inputs = build(&state, None, None, &BTreeMap::new());
    assert_eq!(inputs[INPUT_PROMPT], json!(TRIMMED_BODY));
}

fn state(source: Source, track: Option<Track>) -> FlowState {
    let def = FlowDef {
        id: FLOW_ID.to_string(),
        version: 1,
        inputs: BTreeMap::from([(
            DECLARED_INPUT.to_string(),
            json!({ DECLARATION_REQUIRED: true }),
        )]),
        ..FlowDef::default()
    };
    let mut state = FlowState::new(KEY, def);
    state.ticket = json!({
        TICKET_KEY_FIELD: KEY,
        TICKET_SOURCE: source,
        TICKET_TRACK: track,
        TICKET_SUMMARY: SUMMARY,
    });
    state
}

fn report_from_verdict() -> BTreeMap<String, Value> {
    BTreeMap::from([(WITH_REPORT_FROM.to_string(), json!(VERDICT_STEP))])
}

#[test]
fn a_github_ticket_runs_in_issue_mode_on_its_number() {
    let inputs = build(
        &state(Source::Github, None),
        Some(BODY),
        None,
        &BTreeMap::new(),
    );
    assert_eq!(inputs[INPUT_ISSUE], json!("7"));
    assert!(!inputs.contains_key(INPUT_PROMPT));
    assert_eq!(inputs[INPUT_FEATURE], json!(KEY));
}

#[test]
fn a_local_ticket_runs_in_prompt_mode_on_its_trimmed_body() {
    let state = state(Source::Local, Some(Track::Quick));
    let inputs = build(&state, Some(BODY), None, &BTreeMap::new());
    assert_eq!(inputs[INPUT_PROMPT], json!(TRIMMED_BODY));
    assert!(!inputs.contains_key(INPUT_ISSUE));
    assert_eq!(inputs[INPUT_TRACK], json!(Track::Quick.as_str()));
}

#[test]
fn a_local_ticket_without_body_runs_on_its_summary() {
    let inputs = build(
        &state(Source::Local, None),
        Some(BLANK),
        None,
        &BTreeMap::new(),
    );
    assert_eq!(inputs[INPUT_PROMPT], json!(SUMMARY));
    assert!(!inputs.contains_key(INPUT_TRACK));
}

#[test]
fn a_report_heads_the_prompt_and_report_from_never_reaches_the_run() {
    let state = state(Source::Local, None);
    let inputs = build(&state, Some(BODY), Some(REPORT), &report_from_verdict());
    assert_eq!(
        inputs[INPUT_PROMPT],
        json!(format!("{REPORT}{PROMPT_SEPARATOR}{TRIMMED_BODY}"))
    );
    assert!(!inputs.contains_key(WITH_REPORT_FROM));
}

#[test]
fn a_github_ticket_keeps_its_issue_and_gets_the_report_as_prompt() {
    let state = state(Source::Github, None);
    let inputs = build(&state, None, Some(REPORT), &report_from_verdict());
    assert_eq!(inputs[INPUT_ISSUE], json!("7"));
    assert_eq!(inputs[INPUT_PROMPT], json!(REPORT));
}

#[test]
fn report_path_reads_the_stored_report_of_the_named_step() {
    let mut state = state(Source::Local, None);
    assert!(report_path(&state, &report_from_verdict()).is_none());
    state.steps.insert(
        VERDICT_STEP.to_string(),
        StepState {
            outputs: BTreeMap::from([(OUTPUT_REPORT.to_string(), json!(REPORT_PATH))]),
            ..StepState::default()
        },
    );
    assert_eq!(
        report_path(&state, &report_from_verdict()),
        Some(REPORT_PATH)
    );
    assert!(report_path(&state, &BTreeMap::new()).is_none());
}

#[test]
fn the_step_with_overrides_the_ticket_but_not_the_feature() {
    let with = BTreeMap::from([
        (INPUT_TRACK.to_string(), json!(WITH_TRACK)),
        (INPUT_FEATURE.to_string(), json!(OTHER_FEATURE)),
    ]);
    let inputs = build(&state(Source::Local, Some(Track::Quick)), None, None, &with);
    assert_eq!(inputs[INPUT_TRACK], json!(WITH_TRACK));
    assert_eq!(inputs[INPUT_FEATURE], json!(KEY));
}

#[test]
fn the_flow_input_declarations_never_reach_the_run() {
    let inputs = build(&state(Source::Github, None), None, None, &BTreeMap::new());
    assert!(!inputs.contains_key(DECLARED_INPUT));
    assert!(inputs.values().all(|value| *value != Value::Null));
}

/// Only a flow that defines the `publish` block tells its runs that the host publishes the branch.
#[test]
fn a_flow_with_a_publish_block_tells_its_runs_the_host_publishes() {
    let mut state = state(Source::Github, None);
    let plain = build(&state, None, None, &BTreeMap::new());
    assert!(!plain.contains_key(INPUT_PUBLISH));
    state
        .def
        .blocks
        .insert(PUBLISH_BLOCK.to_string(), Vec::new());
    let published = build(&state, None, None, &BTreeMap::new());
    assert_eq!(published[INPUT_PUBLISH], json!(PUBLISH_BY_HOST));
}
