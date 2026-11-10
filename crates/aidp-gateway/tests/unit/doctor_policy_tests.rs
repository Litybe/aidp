use super::*;
use crate::models::DoctorActionOutcome;

const ALLOWED_CONFIDENCE: f64 = 0.9;
const LOW_CONFIDENCE: f64 = 0.4;
const BETWEEN: f64 = 0.6;
const FORBIDDEN: [&str; 5] = [
    "merge_pr",
    "approve_step",
    "force_push",
    "rebase",
    "open_unity",
];

fn diagnosis(action: &str, confidence: f64, needs_human: bool) -> DoctorDiagnosis {
    DoctorDiagnosis {
        cause: "the cause is written".to_string(),
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

fn done(actions: &[DoctorAction]) -> Vec<DoctorActionOutcome> {
    actions
        .iter()
        .enumerate()
        .map(|(index, action)| DoctorActionOutcome {
            action: *action,
            idempotency_key: format!("k:{index}"),
            ok: false,
            detail: String::new(),
            at: String::new(),
        })
        .collect()
}

#[test]
fn a_reversible_action_needs_half_the_confidence_the_others_need_seven_tenths() {
    for action in [
        DoctorAction::RetryStep,
        DoctorAction::RequeueToTest,
        DoctorAction::RequeueDev,
    ] {
        assert_eq!(
            decide(&diagnosis(action.as_str(), BETWEEN, false)),
            Decision::Execute(action),
            "{action:?} runs at 0.6"
        );
        assert!(matches!(
            decide(&diagnosis(action.as_str(), LOW_CONFIDENCE, false)),
            Decision::NeedsHuman(_)
        ));
    }
    for action in [
        DoctorAction::AbortDeadFlowAndRequeue,
        DoctorAction::UpdatePrBranch,
        DoctorAction::ReleaseUnityLock,
        DoctorAction::CreateBugTicket,
        DoctorAction::OpenHarnessPr,
    ] {
        assert!(
            matches!(
                decide(&diagnosis(action.as_str(), BETWEEN, false)),
                Decision::NeedsHuman(_)
            ),
            "{action:?} needs 0.7"
        );
    }
}

#[test]
fn none_needs_a_written_reason() {
    let mut silent = diagnosis("none", ALLOWED_CONFIDENCE, false);
    silent.cause = "  ".to_string();
    assert!(matches!(decide(&silent), Decision::NeedsHuman(_)));
    let written = diagnosis("none", ALLOWED_CONFIDENCE, false);
    assert_eq!(decide(&written), Decision::NoAction);
}

#[test]
fn the_escalation_goes_resumption_requeue_then_stops() {
    let category = DoctorCategory::SessionLost;
    assert_eq!(
        next_rung(&done(&[]), category, false),
        Some(DoctorAction::RetryStep)
    );
    assert_eq!(
        next_rung(&done(&[DoctorAction::RetryStep]), category, false),
        Some(DoctorAction::RetryStep)
    );
    let two = [DoctorAction::RetryStep, DoctorAction::RetryStep];
    assert_eq!(
        next_rung(&done(&two), category, false),
        Some(DoctorAction::RequeueDev)
    );
    assert_eq!(
        next_rung(&done(&two), category, true),
        Some(DoctorAction::RequeueToTest)
    );
    let three = [
        DoctorAction::RetryStep,
        DoctorAction::RetryStep,
        DoctorAction::RequeueDev,
    ];
    assert_eq!(next_rung(&done(&three), category, false), None);
}

#[test]
fn a_product_bug_goes_to_the_bug_ticket_once_and_a_harness_defect_to_nobody() {
    assert_eq!(
        next_rung(&done(&[]), DoctorCategory::ProductBug, false),
        Some(DoctorAction::CreateBugTicket)
    );
    assert_eq!(
        next_rung(
            &done(&[DoctorAction::CreateBugTicket]),
            DoctorCategory::ProductBug,
            false
        ),
        None
    );
    assert_eq!(
        next_rung(&done(&[]), DoctorCategory::HarnessDefect, false),
        None
    );
}

#[test]
fn the_wait_between_resumptions_grows() {
    assert_eq!(retry_delay_ms(0), 0);
    assert!(retry_delay_ms(1) > 0);
    assert_eq!(retry_delay_ms(2), 2 * retry_delay_ms(1));
    assert_eq!(retry_delay_ms(3), 2 * retry_delay_ms(2));
}

#[test]
fn an_incident_acts_three_times_at_most() {
    assert!(may_act(&done(&[
        DoctorAction::RetryStep,
        DoctorAction::RetryStep
    ])));
    assert!(!may_act(&done(&[
        DoctorAction::RetryStep,
        DoctorAction::RetryStep,
        DoctorAction::RequeueDev
    ])));
}

#[test]
fn no_allowed_action_touches_a_sprite_or_an_asset() {
    for action in DoctorAction::ALL {
        let name = action.as_str();
        for forbidden in [
            "sprite", "asset", "art", "generate", "retouch", "edit", "pixel",
        ] {
            assert!(!name.contains(forbidden), "{name} names {forbidden}");
        }
    }
    assert!(DoctorAction::parse("generate_sprite").is_none());
    assert!(DoctorAction::parse("edit_asset").is_none());
}

#[test]
fn restoring_the_test_infrastructure_pushes_a_commit_so_it_needs_seven_tenths() {
    assert!(!DoctorAction::RestoreTestInfra.is_reversible());
    assert_eq!(
        required_confidence(DoctorAction::RestoreTestInfra),
        crate::models::constants::doctor::MIN_CONFIDENCE
    );
    assert_eq!(
        DoctorAction::parse("restore_test_infra"),
        Some(DoctorAction::RestoreTestInfra)
    );
    let decision = decide(&diagnosis(
        DoctorAction::RestoreTestInfra.as_str(),
        BETWEEN,
        false,
    ));
    assert!(matches!(decision, Decision::NeedsHuman(_)), "{decision:?}");
}

#[test]
fn a_proven_infrastructure_cause_has_one_action_and_no_rung_after_it() {
    for category in [
        DoctorCategory::TestInfraRegression,
        DoctorCategory::BranchBehindMain,
        DoctorCategory::RealDefect,
    ] {
        assert_eq!(next_rung(&[], category, true), None, "{category:?}");
        assert_eq!(
            next_rung(&done(&[DoctorAction::RetryStep]), category, true),
            None
        );
    }
}

#[test]
fn a_shared_harness_failure_falls_back_to_a_bug_on_the_harness_ticket_once() {
    let category = DoctorCategory::SharedHarnessFailure;
    assert!(escalates(category));
    assert_eq!(next_rung(&[], category, false), None);
    assert_eq!(
        next_rung(&done(&[DoctorAction::OpenHarnessPr]), category, false),
        Some(DoctorAction::CreateBugTicket)
    );
    assert_eq!(
        next_rung(
            &done(&[DoctorAction::OpenHarnessPr, DoctorAction::CreateBugTicket]),
            category,
            false
        ),
        None
    );
    assert_eq!(
        next_rung(&done(&[DoctorAction::CreateBugTicket]), category, false),
        None
    );
    assert!(!escalates(DoctorCategory::ProductBug));
}

#[test]
fn a_real_defect_summary_names_the_defect_the_file_and_the_criterion() {
    let mut proven = diagnosis(DoctorAction::None.as_str(), ALLOWED_CONFIDENCE, true);
    proven.params.insert(
        "defect".to_string(),
        serde_json::json!("le joueur n'est pas instancié"),
    );
    proven.params.insert(
        "file".to_string(),
        serde_json::json!("Assets/Scripts/Spawner.cs"),
    );
    proven
        .params
        .insert("criterion".to_string(), serde_json::json!("CA-2"));
    let summary = real_defect_summary(&proven);
    assert!(
        summary.contains("le joueur n'est pas instancié"),
        "{summary}"
    );
    assert!(summary.contains("Assets/Scripts/Spawner.cs"), "{summary}");
    assert!(summary.contains("CA-2"), "{summary}");
}

#[test]
fn a_real_defect_summary_falls_back_on_the_cause_and_says_what_is_unknown() {
    let bare = diagnosis(DoctorAction::None.as_str(), ALLOWED_CONFIDENCE, true);
    let summary = real_defect_summary(&bare);
    assert!(summary.contains("the cause is written"), "{summary}");
    assert_eq!(summary.matches("inconnu").count(), 2, "{summary}");
}
