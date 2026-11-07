//! Pull-request actions served by the daemon: ready, update-branch, merge and review threads.
//! Each action invalidates the PR status cache and publishes a bus event.

pub use crate::models::PrActionContext;
use crate::models::{
    ActionBody, MergeRequest, ReadyRequest, ThreadRepliedResult, ThreadReplyRequest,
    ThreadsResolveRequest, ThreadsResolvedResult, TicketStatusEvent, UpdateBranchRequest,
};
use crate::prs::{handle_status, ticket_key_of_branch, PrRouteError};
use aidp_integrations::models::merge_outcome::MergeOutcome;
use aidp_integrations::models::ready_outcome::ReadyOutcome;
use aidp_integrations::models::update_outcome::UpdateOutcome;
use aidp_integrations::{pr_actions, IntegrationError};
use aidp_tickets::ledger::{self, Event};
use aidp_tickets::model::{guard, Actor, Status, Ticket};
use aidp_tickets::store::{ListFilter, TicketStore};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use tracing::error;

pub const EVENT_READY_CHANGED: &str = "pr.ready_changed";
pub const EVENT_BRANCH_UPDATED: &str = "pr.branch_updated";
pub const EVENT_MERGED: &str = "pr.merged";
pub const EVENT_THREAD_REPLIED: &str = "pr.thread_replied";
pub const EVENT_THREADS_RESOLVED: &str = "pr.threads_resolved";
pub const EVENT_TICKET_STATUS: &str = "ticket.status";
pub const MERGED_FROM_VIEWER_REASON: &str = "merged-from-viewer";

const REPO_SEPARATOR: char = '/';
const KEEP_BRANCH: bool = false;

fn route_error(error: IntegrationError) -> PrRouteError {
    PrRouteError::from(error)
}

fn repo_name(repo: &str) -> &str {
    repo.rsplit(REPO_SEPARATOR).next().unwrap_or(repo)
}

fn head_ref(ctx: &PrActionContext<'_>, repo: &str, number: u64) -> Option<String> {
    let status = handle_status(ctx.cache, repo, number, false).ok()?;
    status.get("head")?.get("ref")?.as_str().map(str::to_string)
}

/// The ticket carrying the PR: by recorded PR number and service, else by the `feat/<KEY>` head ref.
fn find_ticket(
    ctx: &PrActionContext<'_>,
    store: &TicketStore,
    repo: &str,
    number: u64,
) -> Option<Ticket> {
    let service = repo_name(repo);
    let tickets = store.list(&ListFilter::default());
    let by_number = tickets.iter().find(|ticket| {
        ticket.service.eq_ignore_ascii_case(service)
            && ticket.autopilot.pr.as_ref().and_then(|pr| pr.number) == Some(number)
    });
    if let Some(ticket) = by_number {
        return Some(ticket.clone());
    }
    let branch = head_ref(ctx, repo, number)?;
    let key = ticket_key_of_branch(&branch)?;
    tickets.into_iter().find(|ticket| ticket.key == key)
}

fn open_store(ctx: &PrActionContext<'_>) -> Option<TicketStore> {
    if ctx.vault_root.as_os_str().is_empty() {
        return None;
    }
    TicketStore::open(ctx.vault_root).ok()
}

/// Moves an `in-review` ticket to `done` as the human who merged it, journaled like any transition.
fn move_to_done(ctx: &PrActionContext<'_>, store: &TicketStore, key: &str) -> Option<Ticket> {
    let (mut ticket, body, extra) = store.get(key).ok()?;
    let from = ticket.status;
    if from != Status::InReview || guard(from, Status::Done, Actor::Human, false).is_err() {
        return None;
    }
    ticket.status = Status::Done;
    if let Err(error) = store.save(&mut ticket, &body, &extra) {
        error!("cannot save {key} after merge: {error}");
        return None;
    }
    let event = Event {
        ts: Utc::now(),
        key: key.to_string(),
        from: Some(from),
        to: Status::Done,
        actor: Actor::Human,
        reason: Some(MERGED_FROM_VIEWER_REASON.to_string()),
        session_id: None,
    };
    ledger::append(store.vault_root(), &event);
    let payload = TicketStatusEvent {
        key,
        status: Status::Done.as_str(),
    };
    if let Ok(value) = serde_json::to_value(payload) {
        ctx.bus.emit(EVENT_TICKET_STATUS, value);
    }
    Some(ticket)
}

fn finish<T: Serialize>(
    ctx: &PrActionContext<'_>,
    event: &str,
    repo: &str,
    number: u64,
    result: T,
    ticket: Option<&Ticket>,
) -> Result<Value, PrRouteError> {
    ctx.cache.invalidate(ctx.bus, repo, number);
    let body = ActionBody {
        repo: repo.to_string(),
        number,
        result,
        ticket: ticket.map(|found| found.key.clone()),
    };
    let value = serde_json::to_value(body)
        .map_err(|error| route_error(IntegrationError::Upstream(error.to_string())))?;
    ctx.bus.emit(event, value.clone());
    Ok(value)
}

/// Marks a PR ready for review, or back to draft; only the ready direction touches Jira.
pub fn handle_ready(
    ctx: &PrActionContext<'_>,
    request: ReadyRequest,
) -> Result<Value, PrRouteError> {
    let outcome: ReadyOutcome =
        pr_actions::pr_ready(request.number, Some(&request.repo), request.draft)
            .map_err(route_error)?;
    let store = open_store(ctx);
    let ticket = store
        .as_ref()
        .and_then(|store| find_ticket(ctx, store, &request.repo, request.number));
    finish(
        ctx,
        EVENT_READY_CHANGED,
        &request.repo,
        request.number,
        outcome,
        ticket.as_ref(),
    )
}

/// Rebases the PR branch onto its base.
pub fn handle_update_branch(
    ctx: &PrActionContext<'_>,
    request: UpdateBranchRequest,
) -> Result<Value, PrRouteError> {
    let outcome: UpdateOutcome =
        pr_actions::pr_update_branch(request.number, Some(&request.repo)).map_err(route_error)?;
    let ticket =
        open_store(ctx).and_then(|store| find_ticket(ctx, &store, &request.repo, request.number));
    finish(
        ctx,
        EVENT_BRANCH_UPDATED,
        &request.repo,
        request.number,
        outcome,
        ticket.as_ref(),
    )
}

/// Merges a PR whose head still is `expectedHeadSha`, then closes out its ticket.
pub fn handle_merge(
    ctx: &PrActionContext<'_>,
    request: MergeRequest,
) -> Result<Value, PrRouteError> {
    let outcome: MergeOutcome = pr_actions::pr_merge(
        request.number,
        Some(&request.repo),
        &request.expected_head_sha,
        KEEP_BRANCH,
    )
    .map_err(route_error)?;
    let store = open_store(ctx);
    let mut ticket = store
        .as_ref()
        .and_then(|store| find_ticket(ctx, store, &request.repo, request.number));
    if let (true, Some(store), Some(found)) = (outcome.merged, &store, &ticket) {
        if let Some(moved) = move_to_done(ctx, store, &found.key) {
            ticket = Some(moved);
        }
    }
    finish(
        ctx,
        EVENT_MERGED,
        &request.repo,
        request.number,
        outcome,
        ticket.as_ref(),
    )
}

/// Replies on one review thread.
pub fn handle_thread_reply(
    ctx: &PrActionContext<'_>,
    request: ThreadReplyRequest,
) -> Result<Value, PrRouteError> {
    pr_actions::thread_reply(&request.thread_id, &request.body).map_err(route_error)?;
    let result = ThreadRepliedResult {
        thread_id: request.thread_id,
        replied: true,
    };
    finish(
        ctx,
        EVENT_THREAD_REPLIED,
        &request.repo,
        request.number,
        result,
        None,
    )
}

/// Resolves (or unresolves) review threads; each id reports its own result.
pub fn handle_threads_resolve(
    ctx: &PrActionContext<'_>,
    request: ThreadsResolveRequest,
) -> Result<Value, PrRouteError> {
    let results = pr_actions::thread_resolve(&request.thread_ids, request.unresolve);
    let result = ThreadsResolvedResult {
        unresolve: request.unresolve,
        results,
    };
    finish(
        ctx,
        EVENT_THREADS_RESOLVED,
        &request.repo,
        request.number,
        result,
        None,
    )
}

#[cfg(test)]
#[path = "../tests/unit/pr_actions_tests.rs"]
mod tests;
