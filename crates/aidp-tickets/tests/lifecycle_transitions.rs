//! Integration test: every dispatcher transition in `aidp_tickets::lifecycle`
//! against the crate's public API (vision §6 / §7.2). One table-driven test
//! drives all twelve documented transitions through a shared row structure so
//! a broken row is easy to identify (the row name is threaded through every
//! assertion message), plus a handful of focused tests for behaviour the
//! table doesn't cover — session bookkeeping on `on_started`, the
//! non-transition `PrState::Open`, transition legality against
//! `Status::can_transition`, and event attribution.
//!
//! No `#[cfg(test)]` module trick is needed here (unlike
//! `rust/crates/aidp-tickets/tests/jira_sync_mock.rs`, which has to guard a
//! single `#[test]` against process-global env vars): every function under
//! test here is pure, so ordinary parallel `cargo test` threads never
//! interfere with each other.

use chrono::Utc;

use aidp_tickets::config::DispatcherConfig;
use aidp_tickets::ledger::Event;
use aidp_tickets::lifecycle::{self, PrInfo, PrState, SessionOutcome};
use aidp_tickets::model::{Actor, Autopilot, IssueType, Priority, Source, Status, Ticket};

const PR_URL: &str = "https://github.com/example/aidp-viewer/pull/42";

fn sample_pr() -> PrInfo {
    PrInfo {
        url: PR_URL.to_string(),
        number: Some(42),
        state: Some("open".to_string()),
    }
}

/// A minimal ticket in `status`, with `autopilot.attempts` and a live
/// `session_id` ("s-1", the session a dispatcher would have recorded via
/// `on_started`) pre-set — every row below starts from a ticket a session is
/// already attached to, since every transition it exercises is one the
/// dispatcher makes about a session it is tracking.
fn base_ticket(status: Status, attempts: u32) -> Ticket {
    let now = Utc::now();
    Ticket {
        key: "LOC-1".to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "Sample ticket".to_string(),
        status,
        priority: Priority::Medium,
        priority_rank: 0,
        service: "aidp-viewer".to_string(),
        template: None,
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created: now,
        updated: now,
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot: Autopilot {
            attempts,
            session_id: Some("s-1".to_string()),
            ..Default::default()
        },
        tags: Vec::new(),
    }
}

/// One row of the transition table: a starting ticket, the call to make on
/// it, and the expected resulting status / `blocked_reason` / event reason
/// (vision §6 / §7.2). `check` carries whatever extra assertion the row needs
/// beyond that common shape (an `autopilot.attempts` charge, a filled
/// `autopilot.pr`, a retained `session_id`, ...).
struct Row {
    name: &'static str,
    ticket: Ticket,
    call: Box<dyn Fn(&mut Ticket) -> Event>,
    expected_to: Status,
    expected_blocked_reason: Option<&'static str>,
    expected_reason: &'static str,
    check: Box<dyn Fn(&Ticket)>,
}

/// The twelve documented transitions, freshly built on every call so each
/// test that consumes the table gets its own untouched `Ticket`s.
fn transition_table() -> Vec<Row> {
    vec![
        Row {
            name: "done + PR",
            ticket: base_ticket(Status::InProgress, 0),
            call: Box::new(|t| {
                let cfg = DispatcherConfig::default();
                lifecycle::on_ended(t, SessionOutcome::Done, Some(sample_pr()), &cfg)
            }),
            expected_to: Status::InReview,
            expected_blocked_reason: None,
            expected_reason: "pr",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.attempts, 0,
                    "[done + PR] attempts must stay unchanged"
                );
                assert_eq!(
                    t.autopilot.session_id, None,
                    "[done + PR] session_id must be cleared: the session is gone"
                );
                let pr = t
                    .autopilot
                    .pr
                    .as_ref()
                    .expect("[done + PR] autopilot.pr must be filled");
                assert_eq!(
                    pr.url, PR_URL,
                    "[done + PR] autopilot.pr.url must be the PR url"
                );
            }),
        },
        Row {
            name: "done without PR",
            ticket: base_ticket(Status::InProgress, 0),
            call: Box::new(|t| {
                let cfg = DispatcherConfig::default();
                lifecycle::on_ended(t, SessionOutcome::Done, None, &cfg)
            }),
            expected_to: Status::Blocked,
            expected_blocked_reason: Some("no-pr"),
            expected_reason: "no-pr",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.attempts, 0,
                    "[done without PR] attempts must stay unchanged"
                );
                assert_eq!(
                    t.autopilot.session_id, None,
                    "[done without PR] session_id must be cleared"
                );
            }),
        },
        Row {
            name: "failed 1 of 2",
            ticket: base_ticket(Status::InProgress, 0),
            call: Box::new(|t| {
                let cfg = DispatcherConfig {
                    max_attempts: 2,
                    ..DispatcherConfig::default()
                };
                lifecycle::on_ended(
                    t,
                    SessionOutcome::Failed {
                        rate_limited: false,
                    },
                    None,
                    &cfg,
                )
            }),
            expected_to: Status::Todo,
            expected_blocked_reason: None,
            expected_reason: "failed",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.attempts, 1,
                    "[failed 1 of 2] attempts must be charged for the failed attempt"
                );
            }),
        },
        Row {
            name: "failed 2 of 2",
            ticket: base_ticket(Status::InProgress, 1),
            call: Box::new(|t| {
                let cfg = DispatcherConfig {
                    max_attempts: 2,
                    ..DispatcherConfig::default()
                };
                lifecycle::on_ended(
                    t,
                    SessionOutcome::Failed {
                        rate_limited: false,
                    },
                    None,
                    &cfg,
                )
            }),
            expected_to: Status::Blocked,
            expected_blocked_reason: Some("failed"),
            expected_reason: "failed",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.attempts, 2,
                    "[failed 2 of 2] attempts must be charged, exhausting the budget"
                );
            }),
        },
        Row {
            name: "rate limited",
            ticket: base_ticket(Status::InProgress, 1),
            call: Box::new(|t| {
                let cfg = DispatcherConfig {
                    max_attempts: 2,
                    ..DispatcherConfig::default()
                };
                lifecycle::on_ended(t, SessionOutcome::Failed { rate_limited: true }, None, &cfg)
            }),
            expected_to: Status::Todo,
            expected_blocked_reason: None,
            expected_reason: "rate-limited",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.attempts, 1,
                    "[rate limited] the one retry that must NOT burn an attempt"
                );
            }),
        },
        Row {
            name: "cancelled",
            ticket: base_ticket(Status::InProgress, 0),
            call: Box::new(|t| {
                let cfg = DispatcherConfig::default();
                lifecycle::on_ended(t, SessionOutcome::Cancelled, None, &cfg)
            }),
            expected_to: Status::Blocked,
            expected_blocked_reason: Some("cancelled-by-user"),
            expected_reason: "cancelled-by-user",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.session_id, None,
                    "[cancelled] session_id must be cleared: the session is gone"
                );
            }),
        },
        Row {
            name: "orphaned with PR",
            ticket: base_ticket(Status::InProgress, 0),
            call: Box::new(|t| {
                let cfg = DispatcherConfig::default();
                lifecycle::on_ended(t, SessionOutcome::Orphaned, Some(sample_pr()), &cfg)
            }),
            expected_to: Status::InReview,
            expected_blocked_reason: None,
            expected_reason: "orphaned-pr",
            check: Box::new(|t| {
                let pr = t
                    .autopilot
                    .pr
                    .as_ref()
                    .expect("[orphaned with PR] autopilot.pr must be filled");
                assert_eq!(
                    pr.url, PR_URL,
                    "[orphaned with PR] autopilot.pr.url must be the PR url"
                );
            }),
        },
        Row {
            name: "orphaned without PR",
            ticket: base_ticket(Status::InProgress, 0),
            call: Box::new(|t| {
                let cfg = DispatcherConfig::default();
                lifecycle::on_ended(t, SessionOutcome::Orphaned, None, &cfg)
            }),
            expected_to: Status::Todo,
            expected_blocked_reason: None,
            expected_reason: "orphaned",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.attempts, 0,
                    "[orphaned without PR] attempts must be unchanged: an orphaned \
                     daemon restart is not the ticket's fault"
                );
            }),
        },
        Row {
            name: "gate",
            ticket: base_ticket(Status::InProgress, 0),
            call: Box::new(|t| lifecycle::on_gate(t, "pr")),
            expected_to: Status::Blocked,
            expected_blocked_reason: Some("gate:pr"),
            expected_reason: "gate",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.session_id.as_deref(),
                    Some("s-1"),
                    "[gate] the session keeps running and keeps its slot"
                );
            }),
        },
        Row {
            name: "gate answered",
            ticket: {
                let mut t = base_ticket(Status::Blocked, 0);
                t.autopilot.blocked_reason = Some("gate:pr".to_string());
                t
            },
            call: Box::new(lifecycle::on_gate_answered),
            expected_to: Status::InProgress,
            expected_blocked_reason: None,
            expected_reason: "gate-answered",
            check: Box::new(|t| {
                assert_eq!(
                    t.autopilot.session_id.as_deref(),
                    Some("s-1"),
                    "[gate answered] session_id is left untouched: it never changed \
                     while the ticket sat blocked on the gate"
                );
            }),
        },
        Row {
            name: "merged",
            ticket: base_ticket(Status::InReview, 0),
            call: Box::new(|t| {
                lifecycle::on_pr_state(t, PrState::Merged)
                    .expect("[merged] PrState::Merged must produce an Event")
            }),
            expected_to: Status::Done,
            expected_blocked_reason: None,
            expected_reason: "pr-merged",
            check: Box::new(|_t| {}),
        },
        Row {
            name: "closed",
            ticket: base_ticket(Status::InReview, 0),
            call: Box::new(|t| {
                lifecycle::on_pr_state(t, PrState::Closed)
                    .expect("[closed] PrState::Closed must produce an Event")
            }),
            expected_to: Status::Blocked,
            expected_blocked_reason: Some("pr-closed"),
            expected_reason: "pr-closed",
            check: Box::new(|_t| {}),
        },
    ]
}

#[test]
fn every_transition_matches_the_state_machine() {
    for row in transition_table() {
        let Row {
            name,
            mut ticket,
            call,
            expected_to,
            expected_blocked_reason,
            expected_reason,
            check,
        } = row;
        let from = ticket.status;

        let event = call(&mut ticket);

        assert_eq!(
            ticket.status, expected_to,
            "[{name}] resulting status after {from} -> ..."
        );
        assert_eq!(
            ticket.autopilot.blocked_reason.as_deref(),
            expected_blocked_reason,
            "[{name}] blocked_reason"
        );
        assert_eq!(
            event.reason.as_deref(),
            Some(expected_reason),
            "[{name}] event reason"
        );
        assert_eq!(event.to, expected_to, "[{name}] event.to");
        assert_eq!(event.from, Some(from), "[{name}] event.from");
        assert_eq!(event.actor, Actor::Dispatcher, "[{name}] event.actor");

        check(&ticket);
    }
}

#[test]
fn on_started_records_the_session() {
    let mut t = base_ticket(Status::Todo, 0);
    t.autopilot.session_id = None;
    t.autopilot.run_id = None;
    t.autopilot.branch = None;
    // A stale blocked_reason from a previous life of the ticket, to prove
    // on_started clears it on the way in.
    t.autopilot.blocked_reason = Some("stale".to_string());

    let event = lifecycle::on_started(&mut t, "s-1", Some("run-1".to_string()), "feat/LOC-1");

    assert_eq!(t.status, Status::InProgress);
    assert_eq!(t.autopilot.session_id.as_deref(), Some("s-1"));
    assert_eq!(t.autopilot.run_id.as_deref(), Some("run-1"));
    assert_eq!(t.autopilot.branch.as_deref(), Some("feat/LOC-1"));
    assert!(
        t.autopilot.started_at.is_some(),
        "started_at must be recorded"
    );
    assert_eq!(t.autopilot.blocked_reason, None);

    assert_eq!(event.from, Some(Status::Todo));
    assert_eq!(event.to, Status::InProgress);
    assert_eq!(event.actor, Actor::Dispatcher);
    assert_eq!(event.reason.as_deref(), Some("started"));
    assert_eq!(event.session_id.as_deref(), Some("s-1"));
}

#[test]
fn open_pr_is_not_a_transition() {
    let before = base_ticket(Status::InReview, 0);
    let mut after = before.clone();

    let event = lifecycle::on_pr_state(&mut after, PrState::Open);

    assert!(
        event.is_none(),
        "PrState::Open must not produce an Event, got {event:?}"
    );
    assert_eq!(
        after, before,
        "PrState::Open must leave the ticket byte-identical"
    );
}

#[test]
fn every_transition_is_legal_for_the_dispatcher() {
    for row in transition_table() {
        let from = row.ticket.status;
        assert!(
            Status::can_transition(from, row.expected_to, Actor::Dispatcher),
            "[{}] {from} -> {} must be legal for Actor::Dispatcher: the --force \
             rule targets Actor::Human only, so no dispatcher transition may need it",
            row.name,
            row.expected_to
        );
    }
}

/// `Blocked -> Blocked` self-edge, F-review__review_correctness-1: a ticket
/// already blocked on a gate whose session is then cancelled must still get
/// its `session_id`/`blocked_reason` refreshed even though the status number
/// does not move. `Status::allowed_targets(Blocked)` deliberately lists no
/// self-edge for this — `on_ended` applies the refresh unconditionally
/// instead of gating it on `transition_is_legal`, which can never pass a
/// self-edge. Not part of `transition_table()`: its rows are also fed to
/// `every_transition_is_legal_for_the_dispatcher`, which asserts
/// `Status::can_transition(from, to, ..)`, and that must stay `false` for
/// every self-edge.
#[test]
fn cancelled_while_already_blocked_refreshes_the_ticket_in_place() {
    let mut t = base_ticket(Status::Blocked, 0);
    t.autopilot.session_id = Some("s-1".to_string());
    t.autopilot.blocked_reason = Some("gate:pr".to_string());
    let cfg = DispatcherConfig::default();

    let event = lifecycle::on_ended(&mut t, SessionOutcome::Cancelled, None, &cfg);

    assert_eq!(
        t.status,
        Status::Blocked,
        "a Blocked ticket whose gated session is cancelled stays Blocked"
    );
    assert_eq!(
        t.autopilot.session_id, None,
        "session_id must be cleared: the session is gone"
    );
    assert_eq!(
        t.autopilot.blocked_reason.as_deref(),
        Some("cancelled-by-user"),
        "blocked_reason must be overwritten with the new reason"
    );
    assert_eq!(event.from, Some(Status::Blocked));
    assert_eq!(event.to, Status::Blocked);
    assert_eq!(event.reason.as_deref(), Some("cancelled-by-user"));
}

/// `Blocked -> Blocked` self-edge, second reachable path: a gated ticket
/// whose session fails with attempts exhausted must still charge the attempt
/// and refresh `blocked_reason`, even though it was already `Blocked`.
#[test]
fn failed_exhausted_while_already_blocked_refreshes_the_ticket_in_place() {
    let mut t = base_ticket(Status::Blocked, 1);
    t.autopilot.blocked_reason = Some("gate:pr".to_string());
    let cfg = DispatcherConfig {
        max_attempts: 2,
        ..DispatcherConfig::default()
    };

    let event = lifecycle::on_ended(
        &mut t,
        SessionOutcome::Failed {
            rate_limited: false,
        },
        None,
        &cfg,
    );

    assert_eq!(
        t.status,
        Status::Blocked,
        "a Blocked ticket whose session fails out of attempts stays Blocked"
    );
    assert_eq!(
        t.autopilot.attempts, 2,
        "the attempt must still be charged on a same-status refresh"
    );
    assert_eq!(
        t.autopilot.blocked_reason.as_deref(),
        Some("failed"),
        "blocked_reason must be overwritten with the new reason"
    );
    assert_eq!(event.from, Some(Status::Blocked));
    assert_eq!(event.to, Status::Blocked);
    assert_eq!(event.reason.as_deref(), Some("failed"));
}

/// The real, non-self edge added by this fix: a gate-blocked ticket whose
/// session completes with a PR must land `InReview`, not stay stuck illegal.
#[test]
fn done_with_pr_while_blocked_lands_in_review() {
    let mut t = base_ticket(Status::Blocked, 0);
    t.autopilot.blocked_reason = Some("gate:pr".to_string());
    let cfg = DispatcherConfig::default();

    let event = lifecycle::on_ended(&mut t, SessionOutcome::Done, Some(sample_pr()), &cfg);

    assert_eq!(t.status, Status::InReview);
    let pr = t
        .autopilot
        .pr
        .as_ref()
        .expect("autopilot.pr must be filled");
    assert_eq!(pr.url, PR_URL);
    assert_eq!(t.autopilot.blocked_reason, None);
    assert_eq!(event.from, Some(Status::Blocked));
    assert_eq!(event.to, Status::InReview);
    assert!(
        Status::can_transition(Status::Blocked, Status::InReview, Actor::Dispatcher),
        "Blocked -> InReview must now be a legal, listed edge"
    );
}

#[test]
fn illegal_transition_error_downcasts_to_conflict() {
    use aidp_tickets::model::guard;
    use aidp_tickets::TicketError;

    let err = guard(Status::Todo, Status::Done, Actor::Human, true).unwrap_err();
    assert!(err.to_string().contains("not a legal transition"));
    let typed = err
        .downcast_ref::<TicketError>()
        .expect("illegal transition must downcast to TicketError");
    assert!(matches!(typed, TicketError::Conflict(_)));
}

#[test]
fn every_event_is_attributed_to_the_dispatcher() {
    for row in transition_table() {
        let Row {
            name,
            mut ticket,
            call,
            ..
        } = row;
        let from = ticket.status;

        let event = call(&mut ticket);

        assert_eq!(event.actor, Actor::Dispatcher, "[{name}] event.actor");
        assert_eq!(event.from, Some(from), "[{name}] event.from");
    }
}
