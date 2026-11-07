use super::*;

const BASIC: &str = include_str!("../fixtures/basic.yaml");
const LOOP_MAX: &str = include_str!("../fixtures/loop_max.yaml");
const UNKNOWN_FIELD_YAML: &str = "id: x\nversion: 1\nsurprise: true\n";
const TICKET_KEY: &str = "LOC-1";

#[test]
fn parses_every_step_kind_and_field_of_the_basic_fixture() {
    let def = FlowDef::from_yaml(BASIC).unwrap();

    assert_eq!(def.id, "basic");
    assert_eq!(def.version, 1);
    assert_eq!(def.steps.len(), 5);
    assert_eq!(def.steps[1].kind, StepKind::Pipeline);
    assert!(def.steps[1].pushes);
    assert_eq!(def.steps[1].use_.as_deref(), Some("dev"));
    assert_eq!(def.steps[2].kind, StepKind::Human);
    assert_eq!(def.steps[2].choices, vec!["approve", "reject"]);
    assert_eq!(def.steps[4].skip_on, vec!["local"]);
    assert_eq!(def.blocks[CI_BLOCK][0].kind, StepKind::Wait);
}

#[test]
fn parses_loop_fields() {
    let def = FlowDef::from_yaml(LOOP_MAX).unwrap();
    let loop_step = &def.steps[0];

    assert_eq!(loop_step.kind, StepKind::Loop);
    assert_eq!(loop_step.max, Some(2));
    assert_eq!(loop_step.body.len(), 2);
    assert!(loop_step.until.is_some());
}

#[test]
fn defaults_on_error_to_fail() {
    let def = FlowDef::from_yaml(BASIC).unwrap();

    assert!(def.steps.iter().all(|step| step.on_error == OnError::Fail));
}

#[test]
fn rejects_unknown_fields() {
    let error = FlowDef::from_yaml(UNKNOWN_FIELD_YAML).unwrap_err();

    assert!(matches!(error, FlowError::Definition(_)));
}

#[test]
fn contains_human_looks_inside_loop_bodies() {
    let mut outer = Step::new("outer", StepKind::Loop);
    outer.body = vec![Step::new("ask", StepKind::Human)];

    assert!(outer.contains_human());
    assert!(!Step::new("plain", StepKind::Action).contains_human());
}

#[test]
fn new_state_is_fresh_and_copies_settings() {
    let def = FlowDef::from_yaml(BASIC).unwrap();
    let state = FlowState::new(TICKET_KEY, def.clone());

    assert!(state.is_fresh());
    assert_eq!(state.settings, def.settings);
    assert_eq!(state.def.version, def.version);
}

#[test]
fn state_round_trips_through_json() {
    let def = FlowDef::from_yaml(BASIC).unwrap();
    let state = FlowState::new(TICKET_KEY, def);

    let text = serde_json::to_string(&state).unwrap();
    let back: FlowState = serde_json::from_str(&text).unwrap();

    assert_eq!(back, state);
}

#[test]
fn status_wire_values_match_serde() {
    for status in [
        FlowStatus::WaitingHuman,
        FlowStatus::WaitingExternal,
        FlowStatus::Escalated,
    ] {
        let serialized = serde_json::to_value(status).unwrap();
        assert_eq!(serialized, status.as_wire());
    }
    let serialized = serde_json::to_value(StepStatus::WaitingExternal).unwrap();
    assert_eq!(serialized, StepStatus::WaitingExternal.as_wire());
}

#[test]
fn terminal_statuses_are_done_failed_escalated_aborted() {
    assert!(FlowStatus::Done.is_terminal());
    assert!(FlowStatus::Failed.is_terminal());
    assert!(FlowStatus::Escalated.is_terminal());
    assert!(FlowStatus::Aborted.is_terminal());
    assert!(!FlowStatus::Paused.is_terminal());
    assert!(!FlowStatus::Running.is_terminal());
}
