//! Every ticket transition the dispatcher performs on a session event
//! (vision §6 / §7.2).
//!
//! **Pure, no I/O.** Each `on_*` function here mutates the `&mut Ticket` it
//! is given in memory and returns the [`Event`] describing the transition —
//! nothing more. Writing the note (`store::TicketStore::save`), appending
//! the event to the journal (`ledger::append`) and emitting on the bus are
//! always the caller's job, never this module's. That split is deliberate:
//! it is what makes every transition in here unit-testable without a vault,
//! a filesystem, or a bus — see `tests/lifecycle_transitions.rs`.
//!
//! Every event this module builds carries `actor: Actor::Dispatcher` — the
//! `Actor` variant reserved for exactly this caller — `ts: Utc::now()`, the
//! ticket's `key`, `from: Some(<status before the call>)`, and `to`.
//!
//! `to == from` (a same-status result) is a **field refresh**, not a matrix
//! transition: [`on_ended`] applies it unconditionally, without ever
//! consulting the transition matrix. `Status::allowed_targets` never lists a
//! self-edge (see its own doc comment) and is not meant to arbitrate this
//! case — a session ending while the ticket is already `Blocked` still needs
//! its `blocked_reason`/`session_id` refreshed even though the status number
//! does not move.

use crate::ledger::Event;
use crate::model::{Actor, Status, Ticket};
use crate::models::adopted_pr::AdoptedPr;
use crate::models::enums::adoption_reason::AdoptionReason;
pub use crate::models::enums::pr_state::PrState;
use chrono::{DateTime, Utc};

/// How a dispatched session finished, as reported by the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    Done,
    Failed { rate_limited: bool },
    Cancelled,
    Orphaned,
}

/// The pull request an autopilot session opened, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub url: String,
    pub number: Option<u64>,
    pub state: Option<String>,
}

/// Confirm `from -> to` is legal for [`Actor::Dispatcher`] before a
/// transition below applies it.
///
/// Every transition this module performs is legal under
/// [`crate::model::Status::allowed_targets`] by construction (`guard`'s
/// `--force` rule only ever refuses [`Actor::Human`]), so this is a
/// defensive backstop that should never actually trip. A debug build
/// panics on it — a real bug caught early, before it reaches a note or the
/// journal. A release build must not panic a long-running daemon over one
/// ticket: it logs to stderr instead and returns `false`, and the caller
/// leaves the ticket's fields untouched rather than risk an inconsistent
/// status.
fn transition_is_legal(from: Status, to: Status) -> bool {
    match crate::model::guard(from, to, Actor::Dispatcher, false) {
        Ok(()) => true,
        Err(e) => {
            debug_assert!(
                false,
                "aidp-tickets: lifecycle: illegal dispatcher transition {from} -> {to}: {e}"
            );
            tracing::warn!("illegal dispatcher transition {from} -> {to}: {e}");
            false
        }
    }
}

/// A dispatcher session started work on `t`. `-> Status::InProgress`.
pub fn on_started(t: &mut Ticket, session_id: &str, run_id: Option<String>, branch: &str) -> Event {
    let from = t.status;
    let to = Status::InProgress;
    let now = Utc::now();

    if transition_is_legal(from, to) {
        t.status = to;
        t.autopilot.session_id = Some(session_id.to_string());
        t.autopilot.run_id = run_id;
        t.autopilot.branch = Some(branch.to_string());
        t.autopilot.started_at = Some(now);
        t.autopilot.blocked_reason = None;
    }

    Event {
        ts: now,
        key: t.key.clone(),
        from: Some(from),
        to,
        actor: Actor::Dispatcher,
        reason: Some("started".to_string()),
        session_id: Some(session_id.to_string()),
    }
}

/// A dispatcher session for `t` ended. The target status, the ticket's
/// `blocked_reason`, and whether `autopilot.attempts` is charged all depend
/// on `outcome` and on whether a PR came out of the session — see the
/// module's plan task for the full table. `autopilot.session_id` is always
/// cleared: this event means the session is gone, so the slot is free
/// regardless of how it ended.
///
/// When the computed `to` equals the ticket's current status (e.g. a gate
/// still `Blocked` after `Cancelled` or an exhausted `Failed`), the field
/// mutations below are applied unconditionally as a same-status refresh —
/// `transition_is_legal` is never consulted for a self-edge, since it can
/// never pass one and was never meant to. The returned `Event` still carries
/// `from: Some(from), to` with `from == to`: an honest record of "still
/// blocked, reason updated", not a real transition.
pub fn on_ended(
    t: &mut Ticket,
    outcome: SessionOutcome,
    pr: Option<PrInfo>,
    cfg: &crate::config::DispatcherConfig,
) -> Event {
    let from = t.status;
    let now = Utc::now();
    let session_id = t.autopilot.session_id.clone();

    let (to, reason) = match outcome {
        SessionOutcome::Done => {
            if pr.is_some() {
                (Status::InReview, "pr")
            } else {
                (Status::Blocked, "no-pr")
            }
        }
        SessionOutcome::Failed { rate_limited: true } => (Status::Todo, "rate-limited"),
        SessionOutcome::Failed {
            rate_limited: false,
        } => {
            if t.autopilot.attempts + 1 < cfg.max_attempts {
                (Status::Todo, "failed")
            } else {
                (Status::Blocked, "failed")
            }
        }
        SessionOutcome::Cancelled => (Status::Blocked, "cancelled-by-user"),
        SessionOutcome::Orphaned => {
            if pr.is_some() {
                (Status::InReview, "orphaned-pr")
            } else {
                (Status::Todo, "orphaned")
            }
        }
    };

    if to == from || transition_is_legal(from, to) {
        t.status = to;
        t.autopilot.session_id = None;

        match outcome {
            SessionOutcome::Done | SessionOutcome::Orphaned => {
                if let Some(info) = pr {
                    t.autopilot.pr = Some(crate::model::Pr {
                        url: info.url,
                        number: info.number,
                        state: info.state.as_deref().and_then(PrState::from_wire),
                        commented: false,
                    });
                    t.autopilot.blocked_reason = None;
                } else {
                    // Done+no-pr lands on Blocked; Orphaned+no-pr lands on
                    // Todo (the daemon restarting is not the ticket's
                    // fault, so it is not left blocked).
                    t.autopilot.blocked_reason =
                        (to == Status::Blocked).then(|| reason.to_string());
                }
            }
            SessionOutcome::Failed {
                rate_limited: false,
            } => {
                t.autopilot.attempts += 1;
                t.autopilot.blocked_reason = (to == Status::Blocked).then(|| reason.to_string());
            }
            SessionOutcome::Failed { rate_limited: true } => {
                // The one retry that must not burn an attempt or leave the
                // ticket looking blocked.
                t.autopilot.blocked_reason = None;
            }
            SessionOutcome::Cancelled => {
                t.autopilot.blocked_reason = Some(reason.to_string());
            }
        }
    }

    Event {
        ts: now,
        key: t.key.clone(),
        from: Some(from),
        to,
        actor: Actor::Dispatcher,
        reason: Some(reason.to_string()),
        session_id,
    }
}

/// `t` is blocked on a pipeline gate at `stage`. `-> Status::Blocked`.
/// `autopilot.session_id` is deliberately kept: the session is still
/// running and still holds its slot, only waiting on an answer.
pub fn on_gate(t: &mut Ticket, stage: &str) -> Event {
    let from = t.status;
    let to = Status::Blocked;
    let now = Utc::now();
    let blocked_reason = format!("gate:{stage}");
    let session_id = t.autopilot.session_id.clone();

    if transition_is_legal(from, to) {
        t.status = to;
        t.autopilot.blocked_reason = Some(blocked_reason);
        // t.autopilot.session_id untouched on purpose.
    }

    Event {
        ts: now,
        key: t.key.clone(),
        from: Some(from),
        to,
        actor: Actor::Dispatcher,
        reason: Some("gate".to_string()),
        session_id,
    }
}

/// A human answered the gate that was blocking `t`. `-> Status::InProgress`.
/// `autopilot.session_id` is left untouched: it never changed while the
/// ticket sat blocked on the gate.
pub fn on_gate_answered(t: &mut Ticket) -> Event {
    let from = t.status;
    let to = Status::InProgress;
    let now = Utc::now();
    let session_id = t.autopilot.session_id.clone();

    if transition_is_legal(from, to) {
        t.status = to;
        t.autopilot.blocked_reason = None;
    }

    Event {
        ts: now,
        key: t.key.clone(),
        from: Some(from),
        to,
        actor: Actor::Dispatcher,
        reason: Some("gate-answered".to_string()),
        session_id,
    }
}

/// The PR attached to `t` changed state, as observed by the poller.
/// `PrState::Open` is not a transition — nothing about the ticket changes —
/// which is why this returns `None` rather than an `Event`.
pub fn on_pr_state(t: &mut Ticket, state: PrState) -> Option<Event> {
    let from = t.status;
    let now = Utc::now();

    let (to, reason, blocked_reason) = match state {
        PrState::Merged => (Status::Done, "pr-merged", None),
        PrState::Closed => (Status::Blocked, "pr-closed", Some("pr-closed".to_string())),
        PrState::Open => return None,
    };

    if transition_is_legal(from, to) {
        t.status = to;
        t.autopilot.blocked_reason = blocked_reason;
    }

    Some(Event {
        ts: now,
        key: t.key.clone(),
        from: Some(from),
        to,
        actor: Actor::Dispatcher,
        reason: Some(reason.to_string()),
        session_id: None,
    })
}

/// Adopts a PR found before any session: open goes to `in-review`, merged to
/// `done`. Fills `autopilot.pr`, `branch` and `adopted`.
pub fn adopt(
    t: &mut Ticket,
    pr: AdoptedPr,
    actor: Actor,
    now: DateTime<Utc>,
) -> anyhow::Result<Event> {
    let from = t.status;
    let (to, reason) = match (pr.pr.state, actor) {
        (Some(PrState::Merged), _) => (Status::Done, AdoptionReason::AdoptedMerged),
        (_, Actor::Human) => (Status::InReview, AdoptionReason::Adopt),
        _ => (Status::InReview, AdoptionReason::Adopted),
    };
    crate::model::guard_with_reason(from, to, actor, false, Some(reason))?;
    t.status = to;
    t.autopilot.pr = Some(pr.pr);
    t.autopilot.branch = Some(pr.branch);
    t.autopilot.adopted = true;
    t.autopilot.blocked_reason = None;
    Ok(Event {
        ts: now,
        key: t.key.clone(),
        from: Some(from),
        to,
        actor,
        reason: Some(reason.as_str().to_string()),
        session_id: None,
    })
}

#[cfg(test)]
#[path = "../tests/unit/lifecycle_adopt_tests.rs"]
mod lifecycle_adopt_tests;
