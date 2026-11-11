//! The work route (`code`, `art`, `map`, `doc`) of a ticket, for the token ceilings and the model policy.
//! Read from the ticket's `route` field when it has one, then from a `route:<name>` label,
//! then guessed from the template id.

use crate::model::Ticket;
use crate::models::enums::BudgetRoute;
use serde_json::Value;

const ROUTE_FIELD: &str = "route";
const LABEL_PREFIXES: [&str; 3] = ["aidp:route:", "route:", "route/"];
const ART_TEMPLATE_MARKERS: [&str; 1] = ["pixelart"];
const ART_TEMPLATE_PREFIX: &str = "art";
const MAP_TEMPLATE_PREFIX: &str = "map";
const DOC_TEMPLATE_PREFIX: &str = "doc";
const CODE_TEMPLATE_PREFIXES: [&str; 4] = ["dev", "fix", "review", "unity"];

/// The route of `ticket`, `None` when nothing tells.
pub fn route_of_ticket(ticket: &Ticket) -> Option<BudgetRoute> {
    serde_json::to_value(ticket)
        .ok()
        .and_then(|value| route_of_value(&value))
}

/// The route of a ticket serialized as JSON (a flow's ticket snapshot, an API answer).
pub fn route_of_value(ticket: &Value) -> Option<BudgetRoute> {
    ticket
        .get(ROUTE_FIELD)
        .and_then(Value::as_str)
        .and_then(BudgetRoute::parse)
        .or_else(|| route_of_labels(ticket))
        .or_else(|| {
            ticket
                .get("template")
                .and_then(Value::as_str)
                .and_then(route_of_template)
        })
}

fn route_of_labels(ticket: &Value) -> Option<BudgetRoute> {
    ticket
        .get("labels")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|label| {
            LABEL_PREFIXES
                .iter()
                .find_map(|prefix| label.strip_prefix(prefix))
                .and_then(BudgetRoute::parse)
        })
}

/// The route a pipeline template id usually serves.
pub fn route_of_template(template: &str) -> Option<BudgetRoute> {
    let id = template.trim().to_ascii_lowercase();
    if ART_TEMPLATE_MARKERS
        .iter()
        .any(|marker| id.contains(marker))
        || id.starts_with(ART_TEMPLATE_PREFIX)
    {
        Some(BudgetRoute::Art)
    } else if id.starts_with(MAP_TEMPLATE_PREFIX) {
        Some(BudgetRoute::Map)
    } else if id.starts_with(DOC_TEMPLATE_PREFIX) {
        Some(BudgetRoute::Doc)
    } else if CODE_TEMPLATE_PREFIXES
        .iter()
        .any(|prefix| id.starts_with(prefix))
    {
        Some(BudgetRoute::Code)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "../tests/unit/ticket_route_tests.rs"]
mod tests;
