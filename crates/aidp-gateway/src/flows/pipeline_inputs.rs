//! The inputs a flow pipeline step hands its run: the ticket's demand and depth, who publishes
//! the branch, then the step's `with`, then the ticket key as `feature`.

use super::registry::with_as_object;
use crate::models::constants::flow_inputs::{
    GITHUB_BASE_BRANCH, GITHUB_NUMBER, INPUT_BASE_BRANCH, INPUT_FEATURE, INPUT_ISSUE, INPUT_PROMPT,
    INPUT_PUBLISH, INPUT_ROUTE, INPUT_TRACK, PROMPT_SEPARATOR, PUBLISH_BY_HOST, TICKET_GITHUB,
    TICKET_SOURCE, TICKET_SUMMARY, TICKET_TRACK, WITH_REPORT_FROM,
};
use aidp_flows::model::PUBLISH_BLOCK;
use aidp_flows::{FlowState, OUTPUT_REPORT};
use aidp_tickets::model::Source;
use aidp_tickets::Track;
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn ticket_field<T: DeserializeOwned>(ticket: &Value, field: &str) -> Option<T> {
    serde_json::from_value(ticket.get(field)?.clone()).ok()
}

fn non_blank(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

/// Path of the report the step's `with.report_from` names: the stored report of that human step.
pub fn report_path<'a>(state: &'a FlowState, with: &BTreeMap<String, Value>) -> Option<&'a str> {
    let step = with.get(WITH_REPORT_FROM)?.as_str()?;
    state.steps.get(step)?.outputs.get(OUTPUT_REPORT)?.as_str()
}

/// A GitHub mirror runs in issue mode, a local ticket in prompt mode on its body or summary; a `report`
/// heads the prompt, a flow with a `publish` block adds `publish: host`, and the step's `with` wins.
pub fn build(
    state: &FlowState,
    body: Option<&str>,
    report: Option<&str>,
    with: &BTreeMap<String, Value>,
) -> Map<String, Value> {
    let mut inputs = Map::new();
    let mut demand = None;
    match ticket_field::<Source>(&state.ticket, TICKET_SOURCE) {
        Some(Source::Github) => {
            inputs.insert(INPUT_ISSUE.to_string(), json!(issue_number(state)));
        }
        _ => {
            let summary = state.ticket.get(TICKET_SUMMARY).and_then(Value::as_str);
            let description = state
                .ticket
                .get(crate::models::constants::flow_execution::TICKET_DESCRIPTION)
                .and_then(Value::as_str);
            demand = non_blank(body)
                .or_else(|| non_blank(description))
                .or_else(|| non_blank(summary));
        }
    }
    let prompt = match (non_blank(report), demand) {
        (Some(report), Some(demand)) => Some(format!("{report}{PROMPT_SEPARATOR}{demand}")),
        (Some(report), None) => Some(report.to_string()),
        (None, demand) => demand.map(str::to_string),
    };
    if let Some(prompt) = prompt {
        inputs.insert(INPUT_PROMPT.to_string(), json!(prompt));
    }
    if let Some(track) = ticket_field::<Track>(&state.ticket, TICKET_TRACK) {
        inputs.insert(INPUT_TRACK.to_string(), json!(track.as_str()));
    }
    if let Some(route) = aidp_tickets::ticket_route::route_of_value(&state.ticket) {
        inputs.insert(INPUT_ROUTE.to_string(), json!(route.as_str()));
    }
    if let Some(base) = state
        .ticket
        .get(TICKET_GITHUB)
        .and_then(|github| github.get(GITHUB_BASE_BRANCH))
        .and_then(Value::as_str)
        .filter(|base| !base.trim().is_empty())
    {
        inputs.insert(INPUT_BASE_BRANCH.to_string(), json!(base));
    }
    if state.def.blocks.contains_key(PUBLISH_BLOCK) {
        inputs.insert(INPUT_PUBLISH.to_string(), json!(PUBLISH_BY_HOST));
    }
    inputs.extend(with_as_object(with));
    inputs.remove(WITH_REPORT_FROM);
    inputs.insert(INPUT_FEATURE.to_string(), json!(state.key));
    inputs
}

/// The GitHub issue number of a mirror: the `github.number` of its frontmatter, else the number
/// after the last `-` of its key (`GH-12`).
fn issue_number(state: &FlowState) -> String {
    state
        .ticket
        .get(TICKET_GITHUB)
        .and_then(|github| github.get(GITHUB_NUMBER))
        .and_then(Value::as_u64)
        .map(|number| number.to_string())
        .unwrap_or_else(|| {
            let key = ticket_key(state);
            key.rsplit('-').next().unwrap_or(key).to_string()
        })
}

/// The business ticket identity, kept separate from the flow execution's workspace key.
pub fn ticket_key(state: &FlowState) -> &str {
    state
        .ticket
        .get(crate::models::constants::flow_execution::TICKET_KEY)
        .and_then(Value::as_str)
        .unwrap_or(&state.key)
}

#[cfg(test)]
#[path = "../../tests/unit/pipeline_inputs_tests.rs"]
mod tests;
