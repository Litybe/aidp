use super::*;

const ALLOWED_CONFIDENCE: f64 = 0.9;
const LOW_CONFIDENCE: f64 = 0.4;
const FORBIDDEN: [&str; 5] = [
    "merge_pr",
    "approve_step",
    "force_push",
    "rebase",
    "open_unity",
];

fn diagnosis(action: &str, confidence: f64, needs_human: bool) -> DoctorDiagnosis {
    DoctorDiagnosis {
        action: action.to_string(),
        confidence,
        needs_human,
        ..DoctorDiagnosis::default()
    }
}

#[test]
fn every_allowed_action_is_executed_with_enough_confidence() {
    for action in DoctorAction::ALL {
        let expected = if action == DoctorAction::None {
            Decision::NoAction
        } else {
            Decision::Execute(action)
        };
        assert_eq!(
            decide(&diagnosis(action.as_str(), ALLOWED_CONFIDENCE, false)),
            expected
        );
    }
}

#[test]
fn an_action_outside_the_allowed_list_is_refused_whatever_the_confidence() {
    for name in FORBIDDEN {
        let decision = decide(&diagnosis(name, 1.0, false));
        assert!(
            matches!(&decision, Decision::Refused(reason) if reason.contains(name)),
            "{name}: {decision:?}"
        );
    }
}

#[test]
fn a_low_confidence_runs_nothing_and_asks_a_human() {
    let decision = decide(&diagnosis(
        DoctorAction::RetryStep.as_str(),
        LOW_CONFIDENCE,
        false,
    ));
    assert!(matches!(decision, Decision::NeedsHuman(_)));
    let nan = decide(&diagnosis(
        DoctorAction::RetryStep.as_str(),
        f64::NAN,
        false,
    ));
    assert!(matches!(nan, Decision::NeedsHuman(_)));
}

#[test]
fn needs_human_wins_over_the_action() {
    let decision = decide(&diagnosis(
        DoctorAction::RetryStep.as_str(),
        ALLOWED_CONFIDENCE,
        true,
    ));
    assert!(matches!(decision, Decision::NeedsHuman(_)));
}

#[test]
fn the_ticket_cap_counts_investigations() {
    assert!(!capped(0, 2));
    assert!(!capped(1, 2));
    assert!(capped(2, 2));
    assert!(capped(0, 0));
}
