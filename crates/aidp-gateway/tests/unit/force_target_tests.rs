use super::*;

const MERGE_WIRE: &str = "Merge";
const UNKNOWN_WIRE: &str = "shipped";
const AWAITING_MERGE: &str = "awaiting-merge";

#[test]
fn parses_every_wire_form_ignoring_case() {
    for target in ForceTarget::ALL {
        assert_eq!(ForceTarget::parse(target.as_str()), Some(target));
    }
    assert_eq!(ForceTarget::parse(MERGE_WIRE), Some(ForceTarget::Merge));
    assert_eq!(ForceTarget::parse(UNKNOWN_WIRE), None);
}

#[test]
fn merge_lands_in_progress_in_the_test_lane_with_the_awaiting_merge_label() {
    let target = ForceTarget::Merge;
    assert_eq!(target.status(), Status::InProgress);
    assert_eq!(target.phase(), Some(Phase::Testing));
    assert_eq!(target.state_label(), AWAITING_MERGE);
    assert!(target.restarts_flow());
    assert!(target.test_lane_first());
}

#[test]
fn ready_to_test_blocked_and_done_restart_no_flow() {
    for target in [
        ForceTarget::Ready,
        ForceTarget::ToTest,
        ForceTarget::Blocked,
        ForceTarget::Done,
    ] {
        assert!(!target.restarts_flow());
    }
    assert_eq!(ForceTarget::ToTest.phase(), Some(Phase::ToTest));
    assert_eq!(ForceTarget::Done.status(), Status::Done);
}
