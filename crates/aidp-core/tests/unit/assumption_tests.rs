use crate::models::assumption::Assumption;
use crate::run::RunState;

const STAGE_NAME: &str = "plan";
const CHOICE_TEXT: &str = "use the existing helper";
const REASON_TEXT: &str = "smallest change";
const ALTERNATIVE_TEXT: &str = "write a new helper";
const ASSUMPTIONS_KEY: &str = "assumptions";
const AUTONOMOUS_KEY: &str = "autonomous";
const LEGACY_RUN_STATE: &str = include_str!("../fixtures/autonomy/legacy_run_state.json");
const MINIMAL_ASSUMPTION: &str = include_str!("../fixtures/autonomy/assumption.json");

fn sample() -> Assumption {
    Assumption {
        stage: STAGE_NAME.into(),
        choice: CHOICE_TEXT.into(),
        reason: REASON_TEXT.into(),
        rejected_alternative: Some(ALTERNATIVE_TEXT.into()),
        auto_answered: true,
        question_id: None,
        question: None,
    }
}

#[test]
fn assumption_round_trips_through_json() {
    let original = sample();
    let text = serde_json::to_string(&original).unwrap();
    let decoded: Assumption = serde_json::from_str(&text).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn assumption_optional_fields_default() {
    let decoded: Assumption = serde_json::from_str(MINIMAL_ASSUMPTION).unwrap();
    assert_eq!(decoded.rejected_alternative, None);
    assert!(!decoded.auto_answered);
}

#[test]
fn assumption_legacy_run_state_defaults() {
    let state: RunState = serde_json::from_str(LEGACY_RUN_STATE).unwrap();
    assert!(!state.autonomous);
    assert!(state.assumptions.is_empty());
}

#[test]
fn assumption_empty_list_is_not_serialized() {
    let state: RunState = serde_json::from_str(LEGACY_RUN_STATE).unwrap();
    let value = serde_json::to_value(&state).unwrap();
    assert!(value.get(ASSUMPTIONS_KEY).is_none());
    assert!(value.get(AUTONOMOUS_KEY).is_none());
}
