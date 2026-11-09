//! What the daemon accepts from a diagnosis. The agent proposes; this decides.

use crate::models::constants::doctor::MIN_CONFIDENCE;
use crate::models::enums::DoctorAction;
use crate::models::DoctorDiagnosis;

/// The daemon's answer to a diagnosis.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Execute(DoctorAction),
    NoAction,
    NeedsHuman(String),
    Refused(String),
}

/// Checks the proposed action against the allowed list, the human flag and the confidence.
/// A refused name is never executed; a low confidence runs nothing and asks a human.
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
    if diagnosis.confidence.is_nan() || diagnosis.confidence < MIN_CONFIDENCE {
        return Decision::NeedsHuman(format!(
            "confidence {:.2} is below {MIN_CONFIDENCE}",
            diagnosis.confidence
        ));
    }
    match action {
        DoctorAction::None => Decision::NoAction,
        other => Decision::Execute(other),
    }
}

/// True when ticket `ticket` used its investigations of the last 24 hours.
pub fn capped(attempts: usize, max_attempts: u32) -> bool {
    attempts >= max_attempts as usize
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_policy_tests.rs"]
mod tests;
