//! The cost routes' glue: the ticket and run lookups `ticket_cost` leaves to its callers.
//! Behind `GET /api/usage?groupBy=ticket|model|stage|route`, `GET /api/tickets/:key/cost`
//! and `GET /api/cost/anomalies`.

use crate::models::constants::ticket_cost::{GROUP_ROUTE, GROUP_STAGE};
use crate::models::ticket_cost_brief::TicketCostBrief;
use crate::sessions::{RateLimitInfo, SessionRecord};
use crate::ticket_cost::{
    aggregate_contextual, anomalies, run_of, ticket_cost, ticket_of, ticket_totals, AnomalyReport,
    CostContext, TicketCost,
};
use crate::usage::{in_range, UsageReport};
use crate::vault_index::{self, VaultSnapshot};
use aidp_core::models::run_state::RunState;
use aidp_tickets::models::enums::BudgetRoute;
use aidp_tickets::store::{ListFilter, TicketStore};
use aidp_tickets::ticket_route::{route_of_template, route_of_ticket};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// Input of a pipeline run that holds its route (`code`, `art`, `map`, `doc`).
const RUN_ROUTE_INPUT: &str = "route";

/// The route of one run state: its `route` input, else its pipeline id.
pub(crate) fn route_of_state(state: &RunState) -> Option<String> {
    state
        .inputs
        .get(RUN_ROUTE_INPUT)
        .and_then(|value| value.as_str())
        .and_then(BudgetRoute::parse)
        .or_else(|| route_of_template(&state.pipeline_id))
        .map(|route| route.as_str().to_string())
}

/// The route name of a ticket: its note when the vault has one, else the route of its latest
/// run in the index, else its sessions' template.
pub(crate) fn route_of_key_in(
    index: &VaultSnapshot,
    vault: &Path,
    key: &str,
    records: &[SessionRecord],
) -> Option<String> {
    let from_note = (!vault.as_os_str().is_empty())
        .then(|| TicketStore::open(vault).ok())
        .flatten()
        .and_then(|store| store.get(key).ok())
        .and_then(|(ticket, _, _)| route_of_ticket(&ticket))
        .map(|route| route.as_str().to_string());
    from_note.or_else(|| index.route_of(key)).or_else(|| {
        records
            .iter()
            .filter(|record| ticket_of(record) == Some(key))
            .find_map(|record| record.template_id.as_deref().and_then(route_of_template))
            .map(|route| route.as_str().to_string())
    })
}

/// The route of every run that has a session in range, read from the run index.
fn run_routes(
    records: &[SessionRecord],
    from: Option<&str>,
    to: Option<&str>,
    index: &VaultSnapshot,
) -> HashMap<String, String> {
    let mut wanted: Vec<(&str, &str)> = records
        .iter()
        .filter(|record| in_range(&record.started_at, from, to))
        .filter_map(|record| Some((ticket_of(record)?, run_of(record)?)))
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    wanted
        .into_iter()
        .filter_map(|(ticket, run)| Some((run.to_string(), index.route_of_run(ticket, run)?)))
        .collect()
}

/// `GET /api/usage` for the groupings that read tickets, stages or routes.
pub fn usage_report(
    records: &[SessionRecord],
    from: Option<&str>,
    to: Option<&str>,
    group_by: &str,
    vault: &Path,
) -> UsageReport {
    let mut context = CostContext::default();
    let index = vault_index::snapshot(vault);
    if group_by == GROUP_ROUTE {
        context.run_routes = run_routes(records, from, to, &index);
    }
    if group_by == GROUP_STAGE || group_by == GROUP_ROUTE {
        let mut tickets: Vec<&str> = records
            .iter()
            .filter(|record| in_range(&record.started_at, from, to))
            .filter_map(ticket_of)
            .collect();
        tickets.sort_unstable();
        tickets.dedup();
        for key in tickets {
            if group_by == GROUP_STAGE {
                context
                    .stages
                    .insert(key.to_string(), index.stage_rows(key));
            } else if let Some(route) = route_of_key_in(&index, vault, key, records) {
                context.routes.insert(key.to_string(), route);
            }
        }
    }
    aggregate_contextual(records, from, to, group_by, &context)
}

/// `GET /api/tickets/:key/cost`, `None` when no session and no note know the ticket.
pub fn ticket_cost_report(
    key: &str,
    records: &[SessionRecord],
    vault: &Path,
    rate_limit: Option<&RateLimitInfo>,
) -> Option<TicketCost> {
    let has_sessions = records.iter().any(|record| ticket_of(record) == Some(key));
    let has_note = !vault.as_os_str().is_empty()
        && TicketStore::open(vault)
            .ok()
            .is_some_and(|store| store.get(key).is_ok());
    if !has_sessions && !has_note {
        return None;
    }
    let index = vault_index::snapshot(vault);
    let stages = index.stage_rows(key);
    let route = route_of_key_in(&index, vault, key, records);
    let mut report = ticket_cost(key, records, &stages, route, rate_limit);
    if !vault.as_os_str().is_empty() {
        let (budget, pixellab) = crate::ticket_ceiling::report(vault, key, records);
        report.budget = Some(budget);
        report.pixellab = Some(pixellab);
    }
    Some(report)
}

/// `GET /api/tickets/costs`: the dollars and the tokens of each ticket, in one pass over the
/// sessions. `keys` limits the answer; a ticket no session and no note knows is left out.
pub fn ticket_costs_report(
    records: &[SessionRecord],
    vault: &Path,
    keys: Option<&[String]>,
) -> BTreeMap<String, TicketCostBrief> {
    let wanted: Option<HashSet<&str>> = keys.map(|keys| keys.iter().map(String::as_str).collect());
    let mut costs = ticket_totals(records, wanted.as_ref());
    let noted: Vec<String> = if vault.as_os_str().is_empty() {
        Vec::new()
    } else {
        TicketStore::open(vault)
            .map(|store| {
                store
                    .list(&ListFilter {
                        archived: true,
                        ..ListFilter::default()
                    })
                    .into_iter()
                    .map(|ticket| ticket.key)
                    .collect()
            })
            .unwrap_or_default()
    };
    for key in noted {
        if wanted
            .as_ref()
            .is_none_or(|wanted| wanted.contains(key.as_str()))
        {
            costs.entry(key).or_default();
        }
    }
    costs
}

/// `GET /api/cost/anomalies`.
pub fn anomaly_report(
    records: &[SessionRecord],
    from: Option<&str>,
    to: Option<&str>,
) -> AnomalyReport {
    anomalies(records, from, to)
}

#[cfg(test)]
#[path = "../tests/unit/cost_routes_tests.rs"]
mod tests;
