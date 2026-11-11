//! The cost routes' glue: the ticket and run lookups `ticket_cost` leaves to its callers.
//! Behind `GET /api/usage?groupBy=ticket|model|stage|route`, `GET /api/tickets/:key/cost`
//! and `GET /api/cost/anomalies`.

use crate::models::constants::pixellab::PIPELINES_DIR;
use crate::models::constants::ticket_cost::{GROUP_ROUTE, GROUP_STAGE};
use crate::sessions::{RateLimitInfo, SessionRecord};
use crate::ticket_cost::{
    aggregate_contextual, anomalies, load_stage_rows, run_of, ticket_cost, ticket_of,
    AnomalyReport, CostContext, TicketCost,
};
use crate::usage::{in_range, UsageReport};
use aidp_tickets::models::enums::BudgetRoute;
use aidp_tickets::store::TicketStore;
use aidp_tickets::ticket_route::{route_of_template, route_of_ticket};
use std::collections::HashMap;
use std::path::Path;

/// Input of a pipeline run that holds its route (`code`, `art`, `map`, `doc`).
const RUN_ROUTE_INPUT: &str = "route";

/// Directories of the runs of ticket `key` (`<vault>/Pipelines/<repo>/<key>/<run>`), newest first.
fn run_dirs(vault: &Path, key: &str) -> Vec<(String, std::path::PathBuf)> {
    if vault.as_os_str().is_empty() {
        return Vec::new();
    }
    let Ok(repos) = std::fs::read_dir(vault.join(PIPELINES_DIR)) else {
        return Vec::new();
    };
    let mut runs: Vec<(String, std::path::PathBuf)> = repos
        .flatten()
        .filter_map(|repo| std::fs::read_dir(repo.path().join(key)).ok())
        .flat_map(|entries| entries.flatten())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect();
    runs.sort_by(|a, b| b.0.cmp(&a.0));
    runs
}

/// The route of one run: the `route` input of its run state, else its pipeline id.
pub(crate) fn route_of_run_dir(run_dir: &Path) -> Option<String> {
    let state = crate::runs::read_run_state(run_dir)?;
    state
        .inputs
        .get(RUN_ROUTE_INPUT)
        .and_then(|value| value.as_str())
        .and_then(BudgetRoute::parse)
        .or_else(|| route_of_template(&state.pipeline_id))
        .map(|route| route.as_str().to_string())
}

/// The route name of a ticket: its note when the vault has one, else the route of its latest
/// run (run state), else its sessions' template.
pub(crate) fn route_of_key(vault: &Path, key: &str, records: &[SessionRecord]) -> Option<String> {
    let from_note = (!vault.as_os_str().is_empty())
        .then(|| TicketStore::open(vault).ok())
        .flatten()
        .and_then(|store| store.get(key).ok())
        .and_then(|(ticket, _, _)| route_of_ticket(&ticket))
        .map(|route| route.as_str().to_string());
    from_note
        .or_else(|| {
            run_dirs(vault, key)
                .iter()
                .find_map(|(_, dir)| route_of_run_dir(dir))
        })
        .or_else(|| {
            records
                .iter()
                .filter(|record| ticket_of(record) == Some(key))
                .find_map(|record| record.template_id.as_deref().and_then(route_of_template))
                .map(|route| route.as_str().to_string())
        })
}

/// The route of every run that has a session in range, read from the run states of the vault.
fn run_routes(
    records: &[SessionRecord],
    from: Option<&str>,
    to: Option<&str>,
    vault: &Path,
) -> HashMap<String, String> {
    let mut wanted: Vec<(&str, &str)> = records
        .iter()
        .filter(|record| in_range(&record.started_at, from, to))
        .filter_map(|record| Some((ticket_of(record)?, run_of(record)?)))
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    let mut routes = HashMap::new();
    let mut last_ticket: Option<(&str, Vec<(String, std::path::PathBuf)>)> = None;
    for (ticket, run) in wanted {
        if last_ticket.as_ref().map(|(key, _)| *key) != Some(ticket) {
            last_ticket = Some((ticket, run_dirs(vault, ticket)));
        }
        let dirs = last_ticket
            .as_ref()
            .map(|(_, dirs)| dirs.as_slice())
            .unwrap_or(&[]);
        if let Some(route) = dirs
            .iter()
            .find(|(id, _)| id == run)
            .and_then(|(_, dir)| route_of_run_dir(dir))
        {
            routes.insert(run.to_string(), route);
        }
    }
    routes
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
    if group_by == GROUP_ROUTE {
        context.run_routes = run_routes(records, from, to, vault);
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

#[cfg(test)]
#[path = "../tests/unit/cost_routes_tests.rs"]
mod tests;
