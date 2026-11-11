use super::*;
use crate::models::DoctorActionOutcome;

const BETWEEN: f64 = 0.6;
const ALLOWED_CONFIDENCE: f64 = 0.9;

fn diagnosis(action: &str, confidence: f64) -> DoctorDiagnosis {
    DoctorDiagnosis {
        cause: "the cause is written".to_string(),
        action: action.to_string(),
        confidence,
        ..DoctorDiagnosis::default()
    }
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
fn every_category_with_a_dedicated_action_tries_it_first() {
    let expected = [
        (DoctorCategory::FalseFailure, DoctorAction::MarkStepPassed),
        (
            DoctorCategory::RunEndedFlowRunning,
            DoctorAction::ReconcileStep,
        ),
        (
            DoctorCategory::UnityReimportNotFrozen,
            DoctorAction::ExtendUnityWait,
        ),
        (
            DoctorCategory::UnityModalDialog,
            DoctorAction::DismissUnityModal,
        ),
        (DoctorCategory::BudgetWaitSilent, DoctorAction::NudgeBudget),
        (
            DoctorCategory::WorktreeCapSaturated,
            DoctorAction::PruneWorktrees,
        ),
        (
            DoctorCategory::MetaAddAddConflict,
            DoctorAction::ResolveMetaConflict,
        ),
        (
            DoctorCategory::HeadMovedBeforeMerge,
            DoctorAction::RetryMergeStep,
        ),
        (
            DoctorCategory::RebaseConflictOnly,
            DoctorAction::UpdatePrBranch,
        ),
        (
            DoctorCategory::DeadFlowOwnership,
            DoctorAction::AbortDeadFlowAndRequeue,
        ),
    ];
    for (category, action) in expected {
        assert_eq!(dedicated_action(category), Some(action), "{category:?}");
        assert_eq!(
            next_rung(&[], category, false),
            Some(action),
            "{category:?}"
        );
    }
    for category in [
        DoctorCategory::SessionLost,
        DoctorCategory::ApiOverloaded,
        DoctorCategory::McpConnectTimeout,
        DoctorCategory::Other,
    ] {
        assert_eq!(dedicated_action(category), None, "{category:?}");
    }
}

#[test]
fn the_dedicated_rung_comes_before_the_resumptions_and_the_requeue() {
    let category = DoctorCategory::UnityModalDialog;
    let dedicated = DoctorAction::DismissUnityModal;
    assert_eq!(
        next_rung(&done(&[dedicated]), category, false),
        Some(DoctorAction::RetryStep)
    );
    let spent = [dedicated, DoctorAction::RetryStep, DoctorAction::RetryStep];
    assert_eq!(next_rung(&done(&spent), category, true), None);
    let short = [dedicated, DoctorAction::RetryStep];
    assert_eq!(
        next_rung(&done(&short), category, true),
        Some(DoctorAction::RetryStep)
    );
}

#[test]
fn an_overloaded_provider_falls_back_to_another_model_only_when_one_is_set() {
    let category = DoctorCategory::ApiOverloaded;
    let two = done(&[DoctorAction::RetryStep, DoctorAction::RetryStep]);
    let with_fallback = RungOptions {
        fallback_model: true,
    };
    assert_eq!(
        next_rung_with(&two, category, false, with_fallback),
        Some(DoctorAction::RetryWithFallback)
    );
    assert_eq!(
        next_rung_with(&two, category, false, RungOptions::default()),
        Some(DoctorAction::RequeueDev)
    );
    let tried = done(&[
        DoctorAction::RetryStep,
        DoctorAction::RetryStep,
        DoctorAction::RetryWithFallback,
    ]);
    assert_eq!(next_rung_with(&tried, category, false, with_fallback), None);
    assert_eq!(
        next_rung_with(&two, DoctorCategory::SessionLost, false, with_fallback),
        Some(DoctorAction::RequeueDev)
    );
}

#[test]
fn the_new_actions_have_the_confidence_bar_of_their_reach() {
    for action in [
        DoctorAction::ExtendUnityWait,
        DoctorAction::ReconcileStep,
        DoctorAction::NudgeBudget,
    ] {
        assert!(action.is_reversible(), "{action:?}");
        assert!(
            matches!(
                decide(&diagnosis(action.as_str(), BETWEEN)),
                Decision::Execute(_)
            ),
            "{action:?} runs at 0.6"
        );
    }
    for action in [
        DoctorAction::MarkStepPassed,
        DoctorAction::DismissUnityModal,
        DoctorAction::PruneWorktrees,
        DoctorAction::ResolveMetaConflict,
        DoctorAction::RetryMergeStep,
        DoctorAction::KillStaleSession,
        DoctorAction::RetryFromStage,
        DoctorAction::RetryWithFallback,
        DoctorAction::SkipOptionalStep,
        DoctorAction::RebuildBranch,
    ] {
        assert!(!action.is_reversible(), "{action:?}");
        assert!(
            matches!(
                decide(&diagnosis(action.as_str(), BETWEEN)),
                Decision::NeedsHuman(_)
            ),
            "{action:?} needs 0.7"
        );
        assert_eq!(
            decide(&diagnosis(action.as_str(), ALLOWED_CONFIDENCE)),
            Decision::Execute(action)
        );
        assert_eq!(DoctorAction::parse(action.as_str()), Some(action));
    }
}
