//! Tests for the status guard's adoption edges.

use super::*;

/// Asserts the guard accepts the edge for the given actor and adoption reason.
fn assert_guard_ok(from: Status, to: Status, actor: Actor, reason: AdoptionReason) {
    guard_with_reason(from, to, actor, false, Some(reason))
        .unwrap_or_else(|e| panic!("{from:?}->{to:?} {actor:?} {reason:?}: {e}"));
}

/// The dispatcher may move a to-do ticket to review when it adopted a PR.
#[test]
fn guard_accepts_todo_to_in_review_for_adopted_dispatcher() {
    assert_guard_ok(
        Status::Todo,
        Status::InReview,
        Actor::Dispatcher,
        AdoptionReason::Adopted,
    );
}

/// The dispatcher may move a queued ticket to review when it adopted a PR.
#[test]
fn guard_accepts_queued_to_in_review_for_adopted_dispatcher() {
    assert_guard_ok(
        Status::Queued,
        Status::InReview,
        Actor::Dispatcher,
        AdoptionReason::Adopted,
    );
}

/// The dispatcher may close a to-do ticket whose adopted PR is merged.
#[test]
fn guard_accepts_todo_to_done_for_adopted_merged_dispatcher() {
    assert_guard_ok(
        Status::Todo,
        Status::Done,
        Actor::Dispatcher,
        AdoptionReason::AdoptedMerged,
    );
}

/// A human adopting a PR may move a to-do ticket to review.
#[test]
fn guard_accepts_todo_to_in_review_for_human_adopt() {
    assert_guard_ok(
        Status::Todo,
        Status::InReview,
        Actor::Human,
        AdoptionReason::Adopt,
    );
}

/// A done ticket never goes back to review, even when adopted.
#[test]
fn guard_rejects_done_to_in_review_even_when_adopted() {
    let outcome = guard_with_reason(
        Status::Done,
        Status::InReview,
        Actor::Dispatcher,
        false,
        Some(AdoptionReason::Adopted),
    );
    assert!(outcome.is_err());
}

/// Without an adoption reason the to-do to review edge stays closed.
#[test]
fn guard_rejects_todo_to_in_review_without_adoption_reason() {
    let outcome = guard_with_reason(
        Status::Todo,
        Status::InReview,
        Actor::Dispatcher,
        false,
        None,
    );
    assert!(outcome.is_err());
}

/// Frontmatter key of the autopilot delivery flow.
const FLOW_KEY: &str = "flow";

/// Delivery flow name used by the round-trip tests.
const TICKET_DELIVERY_FLOW: &str = "perso-delivery";

/// The flow actor has a stable wire slug and round-trips through serde.
#[test]
fn flow_actor_serializes_to_its_slug() {
    assert_eq!(Actor::Flow.as_str(), FLOW_KEY);
    assert_eq!(Actor::Flow.to_string(), FLOW_KEY);
    assert_eq!(
        serde_yaml::to_string(&Actor::Flow).unwrap().trim(),
        FLOW_KEY
    );
    assert_eq!(
        serde_yaml::from_str::<Actor>(FLOW_KEY).unwrap(),
        Actor::Flow
    );
}

/// An autopilot block carrying a flow round-trips and keeps the flow.
#[test]
fn autopilot_with_flow_round_trips() {
    let autopilot = Autopilot {
        flow: Some(TICKET_DELIVERY_FLOW.to_string()),
        ..Autopilot::default()
    };
    let text = serde_yaml::to_string(&autopilot).unwrap();
    assert!(text.contains(&format!("{FLOW_KEY}: {TICKET_DELIVERY_FLOW}")));
    assert_eq!(serde_yaml::from_str::<Autopilot>(&text).unwrap(), autopilot);
}

/// An autopilot block without a flow never writes the key and round-trips.
#[test]
fn autopilot_without_flow_omits_the_key() {
    let autopilot = Autopilot::default();
    let text = serde_yaml::to_string(&autopilot).unwrap();
    assert!(!text.contains(FLOW_KEY));
    let reparsed: Autopilot = serde_yaml::from_str(&text).unwrap();
    assert_eq!(reparsed.flow, None);
    assert_eq!(serde_yaml::to_string(&reparsed).unwrap(), text);
}
