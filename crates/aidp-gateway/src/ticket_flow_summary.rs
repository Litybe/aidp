//! The `flowSummary` field of the ticket routes: the most recent active flow of a ticket.

use crate::flows::pipeline_inputs::ticket_key;
use crate::flows::FlowEngine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

const FIELD: &str = "flowSummary";
const KEY_FIELD: &str = "key";

/// The summary of a flow state: `{ flowKey, status, phase, stepId, iteration, startedAt }`.
pub fn summarize(state: &aidp_flows::FlowState) -> Value {
    let step = state
        .current
        .as_ref()
        .and_then(|current| state.steps.get(current));
    json!({
        "flowKey": state.key,
        "status": state.status.as_wire(),
        "phase": step.and_then(|s| s.phase.clone()),
        "stepId": state.current,
        "iteration": step.map_or(0, |s| s.iteration),
        "startedAt": state.started_at,
    })
}

/// The newest non-terminal flow of each ticket key.
fn active_by_ticket(engine: &FlowEngine) -> HashMap<String, Value> {
    let mut found = HashMap::new();
    for entry in engine.list(None, None) {
        if entry.status.is_terminal() {
            continue;
        }
        let Ok(state) = engine.get(&entry.key) else {
            continue;
        };
        found
            .entry(ticket_key(&state).to_string())
            .or_insert_with(|| summarize(&state));
    }
    found
}

/// Sets `flowSummary` on every ticket object of a list; `null` when it has no active flow.
pub fn attach_list(engine: Option<&Arc<FlowEngine>>, tickets: &mut Value) {
    let found = engine.map(|e| active_by_ticket(e)).unwrap_or_default();
    for ticket in tickets.as_array_mut().into_iter().flatten() {
        set(ticket, &found);
    }
}

/// Sets `flowSummary` on one ticket object.
pub fn attach_one(engine: Option<&Arc<FlowEngine>>, ticket: &mut Value) {
    let found = engine.map(|e| active_by_ticket(e)).unwrap_or_default();
    set(ticket, &found);
}

fn set(ticket: &mut Value, found: &HashMap<String, Value>) {
    let summary = ticket
        .get(KEY_FIELD)
        .and_then(Value::as_str)
        .and_then(|key| found.get(key))
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(object) = ticket.as_object_mut() {
        object.insert(FIELD.to_string(), summary);
    }
}

#[cfg(test)]
#[path = "../tests/unit/ticket_flow_summary_tests.rs"]
mod tests;
