//! `flowSummary` of the ticket routes: the step fields of a flow, `null` without an active flow.

use super::{attach_list, attach_one, set, summarize};
use aidp_flows::{FlowDef, FlowState, FlowStatus, StepState, StepStatus};
use serde_json::{json, Value};
use std::collections::HashMap;

fn state() -> FlowState {
    let mut state = FlowState::new("GH-7", FlowDef::default());
    state.status = FlowStatus::WaitingHuman;
    state.current = Some("review".to_string());
    state.started_at = Some("2026-11-06T10:00:00Z".to_string());
    state.steps.insert(
        "review".to_string(),
        StepState {
            status: StepStatus::default(),
            iteration: 2,
            phase: Some("code".to_string()),
            ..StepState::default()
        },
    );
    state
}

#[test]
fn summary_carries_the_current_step() {
    assert_eq!(
        summarize(&state()),
        json!({
            "flowKey": "GH-7",
            "status": "waiting_human",
            "phase": "code",
            "stepId": "review",
            "iteration": 2,
            "startedAt": "2026-11-06T10:00:00Z",
        })
    );
}

#[test]
fn a_flow_without_a_current_step_has_null_phase() {
    let mut idle = state();
    idle.current = None;
    let summary = summarize(&idle);
    assert_eq!(summary["phase"], Value::Null);
    assert_eq!(summary["stepId"], Value::Null);
    assert_eq!(summary["iteration"], 0);
}

#[test]
fn a_ticket_gets_its_flow_or_null() {
    let found = HashMap::from([("GH-7".to_string(), summarize(&state()))]);
    let mut with = json!({ "key": "GH-7" });
    let mut without = json!({ "key": "GH-8" });
    set(&mut with, &found);
    set(&mut without, &found);
    assert_eq!(with["flowSummary"]["flowKey"], "GH-7");
    assert_eq!(without["flowSummary"], Value::Null);
}

#[test]
fn without_a_flow_engine_every_ticket_is_null() {
    let mut list = json!([{ "key": "GH-1" }, { "key": "GH-2" }]);
    attach_list(None, &mut list);
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["flowSummary"].is_null() && t.get("flowSummary").is_some()));
    let mut one = json!({ "key": "GH-1" });
    attach_one(None, &mut one);
    assert_eq!(one["flowSummary"], Value::Null);
}
