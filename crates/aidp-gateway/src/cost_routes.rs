//! The cost routes' glue: the ticket and run lookups `ticket_cost` leaves to its callers.
//! Behind `GET /api/usage?groupBy=ticket|model|stage|route`, `GET /api/tickets/:key/cost`
//! and `GET /api/cost/anomalies`.

use crate::models::constants::ticket_cost::{GROUP_ROUTE, GROUP_STAGE};
use crate::sessions::{RateLimitInfo, SessionRecord};
use crate::ticket_cost::{
    aggregate_contextual, anomalies, load_stage_rows, ticket_cost, ticket_of, AnomalyReport,
    CostContext, TicketCost,
};
use crate::usage::{in_range, UsageReport};
use aidp_tickets::store::TicketStore;
use aidp_tickets::ticket_route::{route_of_template, route_of_ticket};
use std::path::Path;

/// The route name of a ticket: its note when the vault has one, else its sessions' template.
pub(crate) fn route_of_key(vault: &Path, key: &str, records: &[SessionRecord]) -> Option<String> {
    let from_note = (!vault.as_os_str().is_empty())
        .then(|| TicketStore::open(vault).ok())
        .flatten()
        .and_then(|store| store.get(key).ok())
        .and_then(|(ticket, _, _)| route_of_ticket(&ticket));
    from_note
        .or_else(|| {
            records
                .iter()
                .filter(|record| ticket_of(record) == Some(key))
                .find_map(|record| record.template_id.as_deref().and_then(route_of_template))
        })
        .map(|route| route.as_str().to_string())
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
                    .insert(key.to_string(), load_stage_rows(vault, key));
            } else if let Some(route) = route_of_key(vault, key, records) {
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
    let stages = if vault.as_os_str().is_empty() {
        Vec::new()
    } else {
        load_stage_rows(vault, key)
    };
    let route = route_of_key(vault, key, records);
    let mut report = ticket_cost(key, records, &stages, route, rate_limit);
    if !vault.as_os_str().is_empty() {
        let (budget, pixellab) = crate::ticket_ceiling::report(vault, key, records);
        report.budget = Some(budget);
        report.pixellab = Some(pixellab);
    }
    Some(report)
}

/// `GET /api/cost/anomalies`.
pub fn anomaly_report(
    records: &[SessionRecord],
    from: Option<&str>,
    to: Option<&str>,
) -> AnomalyReport {
    anomalies(records, from, to)
}
