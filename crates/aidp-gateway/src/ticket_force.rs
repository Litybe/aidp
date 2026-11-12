//! Forced move of a ticket to any board column: the active flow is aborted, the status,
//! phase and state label are written, and a flow restarts from the entry of that column.

use crate::bus::Bus;
use crate::flows::FlowEngine;
use crate::models::constants::ticket_force::{
    EVENT_TICKET_STATUS, FLOWS_UNAVAILABLE, FORCED_BLOCKED_REASON, FORCED_REASON_PREFIX,
    FORCE_ACTOR, REASON_SEPARATOR, UNKNOWN_TARGET,
};
use crate::models::enums::{ApiStatus, ForceTarget};
use crate::models::ticket_flow_request::TicketFlowRequest;
use crate::models::ticket_transition_event::TicketTransitionEvent;
use crate::models::{ApiRefusal, TicketForceReply, TicketForceRequest};
use aidp_core::clock::{Clock, SystemClock};
use aidp_tickets::ledger::{self, Event};
use aidp_tickets::model::{Actor, Ticket};
use aidp_tickets::store::TicketStore;
use std::sync::Arc;

fn refusal(status: ApiStatus, message: impl Into<String>) -> anyhow::Error {
    ApiRefusal {
        status,
        message: message.into(),
    }
    .into()
}

/// The flows of the ticket's route a target may restart, in the order they are tried.
fn route_flows(store: &TicketStore, ticket: &Ticket, target: ForceTarget) -> Vec<String> {
    let routing = store.routing();
    let delivery = routing
        .flow_for(&ticket.service)
        .map(str::to_string)
        .or_else(|| ticket.flow.clone())
        .or_else(|| ticket.autopilot.flow.clone());
    let test = delivery
        .as_deref()
        .and_then(|flow| routing.test_flow_for(flow, ticket.route.as_deref()))
        .map(str::to_string);
    let ordered = if target.test_lane_first() {
        [test, delivery]
    } else {
        [delivery, test]
    };
    ordered.into_iter().flatten().collect()
}

/// The flow to restart and its entry, refused when no flow of the route declares the entry.
fn flow_to_restart(
    store: &TicketStore,
    flows: Option<&Arc<FlowEngine>>,
    ticket: &Ticket,
    target: ForceTarget,
) -> anyhow::Result<Option<String>> {
    if !target.restarts_flow() {
        return Ok(None);
    }
    let engine = flows.ok_or_else(|| refusal(ApiStatus::Config, FLOWS_UNAVAILABLE))?;
    let candidates = route_flows(store, ticket, target);
    let names: Vec<&str> = candidates.iter().map(String::as_str).collect();
    engine
        .flow_with_entry(&names, target.as_str())
        .map(|name| Some(name.to_string()))
        .ok_or_else(|| {
            refusal(
                ApiStatus::Conflict,
                format!(
                    "no flow of the route of `{}` ({}) declares the entry `{}`",
                    ticket.key,
                    names.join(", "),
                    target.as_str()
                ),
            )
        })
}

fn journal_reason(target: ForceTarget, reason: Option<&str>) -> String {
    let forced = format!("{FORCED_REASON_PREFIX}{}", target.as_str());
    match reason.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => format!("{forced}{REASON_SEPARATOR}{text}"),
        None => forced,
    }
}

/// Moves `key` to the column `request.target`, whatever its status, and makes it valid
/// there: the next steps of its path run as if the skipped ones had passed.
pub fn force_ticket(
    store: &TicketStore,
    bus: &Bus,
    flows: Option<&Arc<FlowEngine>>,
    key: &str,
    request: &TicketForceRequest,
) -> anyhow::Result<TicketForceReply> {
    let target = ForceTarget::parse(&request.target)
        .ok_or_else(|| refusal(ApiStatus::BadRequest, UNKNOWN_TARGET))?;
    let (mut ticket, body, extra) = store.get(key)?;
    let flow = flow_to_restart(store, flows, &ticket, target)?;
    let aborted = match flows.filter(|engine| engine.has_active_run(key)) {
        Some(engine) => engine.abort(key, FORCE_ACTOR).map(|_| true)?,
        None => false,
    };
    let now = SystemClock.now_utc();
    let from = ticket.status;
    let to = target.status();
    ticket.status = to;
    if let Some(phase) = target.phase() {
        ticket.autopilot.set_phase(phase, now);
    }
    ticket.autopilot.blocked_reason =
        (target == ForceTarget::Blocked).then(|| FORCED_BLOCKED_REASON.to_string());
    if let Some(name) = flow.as_deref() {
        ticket.autopilot.flow = Some(name.to_string());
        ticket.autopilot.started_at = Some(now);
    }
    store.save(&mut ticket, &body, &extra)?;
    let reason = journal_reason(target, request.reason.as_deref());
    ledger::append(
        store.vault_root(),
        &Event {
            ts: now,
            key: key.to_string(),
            from: Some(from),
            to,
            actor: Actor::Human,
            reason: Some(reason.clone()),
            session_id: None,
        },
    );
    let event = TicketTransitionEvent {
        key,
        status: to.as_str(),
        from: from.as_str(),
        to: to.as_str(),
        reason: Some(&reason),
        actor: Actor::Human.as_str(),
    };
    bus.emit(EVENT_TICKET_STATUS, serde_json::to_value(event)?);
    if ticket.github.is_some() {
        let port = aidp_integrations::github_issues::GhIssuePort::new();
        if let Err(error) =
            aidp_tickets::github_sync::label_ticket(store, &port, key, target.state_label())
        {
            tracing::warn!(key, %error, "forced state label left for the next write-back");
        }
    }
    if let (Some(name), Some(engine)) = (flow.as_deref(), flows) {
        let (fresh, _, _) = store.get(key)?;
        engine.start_request(TicketFlowRequest {
            ticket: serde_json::to_value(&fresh)?,
            flow: Some(name.to_string()),
            entry: Some(target.as_str().to_string()),
            ..TicketFlowRequest::default()
        })?;
    }
    Ok(TicketForceReply {
        key: key.to_string(),
        target: target.as_str().to_string(),
        from: from.as_str().to_string(),
        to: to.as_str().to_string(),
        phase: target.phase().map(|phase| phase.as_str().to_string()),
        aborted,
        entry: flow.as_ref().map(|_| target.as_str().to_string()),
        flow,
    })
}
