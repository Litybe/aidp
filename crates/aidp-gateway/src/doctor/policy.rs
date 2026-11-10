//! What the daemon accepts from a diagnosis. The agent proposes; this decides.

use crate::models::constants::doctor::{
    MAX_ACTIONS_PER_INCIDENT, MAX_TRANSIENT_RETRIES, MIN_CONFIDENCE, MIN_CONFIDENCE_REVERSIBLE,
    PARAM_CRITERION, PARAM_DEFECT, PARAM_DEFECT_FILE, RETRY_BACKOFF_BASE_MS, UNKNOWN_FIELD,
};
use crate::models::enums::{DoctorAction, DoctorCategory};
use crate::models::{DoctorActionOutcome, DoctorDiagnosis};

/// The daemon's answer to a diagnosis.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Execute(DoctorAction),
    NoAction,
    NeedsHuman(String),
    Refused(String),
}

/// The confidence an action needs: lower for the ones that undo themselves.
pub fn required_confidence(action: DoctorAction) -> f64 {
    if action.is_reversible() {
        MIN_CONFIDENCE_REVERSIBLE
    } else {
        MIN_CONFIDENCE
    }
}

/// Checks the proposed action against the allowed list, the human flag and the confidence.
/// A refused name is never executed; a low confidence runs nothing and asks a human;
/// `none` needs a cause written down.
pub fn decide(diagnosis: &DoctorDiagnosis) -> Decision {
    let Some(action) = DoctorAction::parse(&diagnosis.action) else {
        return Decision::Refused(format!(
            "action `{}` is not in the allowed list",
            diagnosis.action
        ));
    };
    if diagnosis.needs_human {
        return Decision::NeedsHuman("the diagnosis asks for a human".to_string());
    }
    let required = required_confidence(action);
    if diagnosis.confidence.is_nan() || diagnosis.confidence < required {
        return Decision::NeedsHuman(format!(
            "confidence {:.2} is below {required}",
            diagnosis.confidence
        ));
    }
    match action {
        DoctorAction::None if diagnosis.cause.trim().is_empty() => {
            Decision::NeedsHuman("`none` without a written reason".to_string())
        }
        DoctorAction::None => Decision::NoAction,
        other => Decision::Execute(other),
    }
}

/// True when the category is one whose action may fall back to another after it did not work.
pub fn escalates(category: DoctorCategory) -> bool {
    category == DoctorCategory::SharedHarnessFailure
}

/// One line that names the defect, the file and the criterion of a `real_defect` diagnosis, for the
/// human the doctor hands the ticket to.
pub fn real_defect_summary(diagnosis: &DoctorDiagnosis) -> String {
    let field = |name: &str| {
        diagnosis
            .params
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or(UNKNOWN_FIELD)
            .to_string()
    };
    let defect = match field(PARAM_DEFECT).as_str() {
        UNKNOWN_FIELD => diagnosis.cause.trim().to_string(),
        named => named.to_string(),
    };
    format!(
        "défaut : {defect} ; fichier : {} ; critère : {}",
        field(PARAM_DEFECT_FILE),
        field(PARAM_CRITERION)
    )
}

/// True when ticket `ticket` used its investigations of the last 24 hours.
pub fn capped(attempts: usize, max_attempts: u32) -> bool {
    attempts >= max_attempts as usize
}

/// True while the incident may still take an action.
pub fn may_act(done: &[DoctorActionOutcome]) -> bool {
    done.len() < MAX_ACTIONS_PER_INCIDENT
}

fn count(done: &[DoctorActionOutcome], action: DoctorAction) -> usize {
    done.iter()
        .filter(|outcome| outcome.action == action)
        .count()
}

/// Milliseconds to wait before the next resumption, given the resumptions already made:
/// none before the first, then 2 min, 4 min, 8 min...
pub fn retry_delay_ms(retries_done: usize) -> i64 {
    match retries_done {
        0 => 0,
        n => RETRY_BACKOFF_BASE_MS << (n - 1).min(10),
    }
}

/// The next rung of the escalation, in order: resumption, requeue (test or dev lane),
/// then nothing: a human. A product bug goes straight to the bug ticket. A rung already taken is skipped
/// except the resumption, which repeats up to its cap. `None` when no rung is left or the
/// incident used its actions.
pub fn next_rung(
    done: &[DoctorActionOutcome],
    category: DoctorCategory,
    test_lane: bool,
) -> Option<DoctorAction> {
    if !may_act(done) {
        return None;
    }
    // A failure of the product or of the harness is not cured by a resumption or a requeue.
    if category == DoctorCategory::ProductBug {
        return (count(done, DoctorAction::CreateBugTicket) == 0)
            .then_some(DoctorAction::CreateBugTicket);
    }
    if category == DoctorCategory::HarnessDefect {
        return None;
    }
    if category == DoctorCategory::SharedHarnessFailure {
        let patched = count(done, DoctorAction::OpenHarnessPr) > 0;
        return (patched && count(done, DoctorAction::CreateBugTicket) == 0)
            .then_some(DoctorAction::CreateBugTicket);
    }
    if matches!(
        category,
        DoctorCategory::TestInfraRegression
            | DoctorCategory::BranchBehindMain
            | DoctorCategory::RealDefect
    ) {
        return None;
    }
    if count(done, DoctorAction::RetryStep) < MAX_TRANSIENT_RETRIES {
        return Some(DoctorAction::RetryStep);
    }
    let requeue = if test_lane {
        DoctorAction::RequeueToTest
    } else {
        DoctorAction::RequeueDev
    };
    if count(done, requeue) == 0 {
        return Some(requeue);
    }
    None
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_policy_tests.rs"]
mod tests;
