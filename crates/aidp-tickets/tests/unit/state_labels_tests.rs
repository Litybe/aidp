//! Tests for the state label write-back and read-back.

use super::*;
use crate::model::Source;

#[path = "test_support.rs"]
#[allow(clippy::duplicate_mod)]
mod test_support;

use test_support::ticket;

fn labels(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn with_status(status: Status, reason: Option<&str>) -> Ticket {
    let mut t = ticket("GH-1", Source::Github);
    t.status = status;
    t.autopilot.blocked_reason = reason.map(str::to_string);
    t
}

#[test]
fn every_status_maps_to_its_label_of_plan_3_1() {
    let cfg = GithubConfig::default();
    let cases = [
        (Status::Todo, None, "aidp:ready"),
        (Status::Queued, None, "aidp:queued"),
        (Status::InReview, None, "aidp:awaiting-merge"),
        (Status::Done, None, "aidp:done"),
        (Status::Blocked, Some("failed"), "aidp:blocked"),
        (Status::Blocked, Some("needs-refine"), "aidp:triage"),
        (Status::Blocked, Some("needs-routing"), "aidp:triage"),
        (Status::Blocked, Some("skip"), "aidp:skip"),
        (Status::Blocked, Some("label:aidp:fixing"), "aidp:fixing"),
    ];
    for (status, reason, label) in cases {
        assert_eq!(
            wanted(&with_status(status, reason), &cfg),
            WantedLabel::Exactly(label.to_string()),
            "{status:?} {reason:?}"
        );
    }
    assert_eq!(
        wanted(&with_status(Status::Cancelled, None), &cfg),
        WantedLabel::Keep
    );
    assert_eq!(
        wanted(&with_status(Status::Blocked, Some("not-ready")), &cfg),
        WantedLabel::Nothing
    );
}

#[test]
fn a_transition_replaces_the_single_state_label() {
    let cfg = GithubConfig::default();
    let current = labels(&["dev", "aidp:ready"]);
    let change = edit(
        &current,
        &wanted(&with_status(Status::Queued, None), &cfg),
        &cfg,
    )
    .expect("the label changes");
    assert_eq!(change.add, labels(&["aidp:queued"]));
    assert_eq!(change.remove, labels(&["aidp:ready"]));
    assert_eq!(apply(&current, &change), labels(&["dev", "aidp:queued"]));
}

#[test]
fn every_extra_state_label_is_removed() {
    let cfg = GithubConfig::default();
    let current = labels(&["aidp:done", "pixelart", "aidp:need-fix", "aidp:in-progress"]);
    let change = edit(
        &current,
        &wanted(&with_status(Status::InReview, None), &cfg),
        &cfg,
    )
    .unwrap();
    assert_eq!(change.add, labels(&["aidp:awaiting-merge"]));
    assert_eq!(
        change.remove,
        labels(&["aidp:done", "aidp:need-fix", "aidp:in-progress"])
    );
}

#[test]
fn in_progress_accepts_a_flow_phase_label() {
    let cfg = GithubConfig::default();
    let in_progress = wanted(&with_status(Status::InProgress, None), &cfg);
    assert_eq!(edit(&labels(&["aidp:review"]), &in_progress, &cfg), None);
    assert_eq!(
        edit(&labels(&["aidp:in-progress"]), &in_progress, &cfg),
        None
    );
    let from_ready = edit(&labels(&["aidp:ready"]), &in_progress, &cfg).unwrap();
    assert_eq!(from_ready.add, labels(&["aidp:in-progress"]));
}

#[test]
fn the_right_label_already_there_needs_no_edit() {
    let cfg = GithubConfig::default();
    let todo = wanted(&with_status(Status::Todo, None), &cfg);
    assert_eq!(edit(&labels(&["bug", "AIDP:READY"]), &todo, &cfg), None);
}

#[test]
fn not_ready_removes_every_state_label() {
    let cfg = GithubConfig::default();
    let change = edit(
        &labels(&["aidp:triage", "dev"]),
        &WantedLabel::Nothing,
        &cfg,
    )
    .unwrap();
    assert!(change.add.is_empty());
    assert_eq!(change.remove, labels(&["aidp:triage"]));
}

#[test]
fn labels_read_back_as_intents() {
    let cfg = GithubConfig::default();
    assert_eq!(intent(None, &cfg), LabelIntent::Unlabelled);
    assert_eq!(intent(Some("aidp:ready"), &cfg), LabelIntent::Ready);
    assert_eq!(intent(Some("aidp:queued"), &cfg), LabelIntent::Ready);
    assert_eq!(intent(Some("aidp:triage"), &cfg), LabelIntent::Triage);
    assert_eq!(intent(Some("aidp:skip"), &cfg), LabelIntent::Skip);
    assert_eq!(
        intent(Some("aidp:fixing"), &cfg),
        LabelIntent::Held("aidp:fixing".to_string())
    );
    assert_eq!(
        current(&labels(&["dev", "aidp:done"]), &cfg),
        Some("aidp:done".to_string())
    );
}
