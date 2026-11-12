//! A dev/bug flow parked on a human gate or paused holds no lane slot.

use super::waits_on_merge;
use aidp_flows::{FlowDef, FlowState, FlowStatus, StepState};

fn parked(status: FlowStatus, phase: &str) -> FlowState {
    let mut state = FlowState::new("GH-9", FlowDef::default());
    state.status = status;
    state.current = Some("merge_approval".to_string());
    state.steps.insert(
        "merge_approval".to_string(),
        StepState {
            phase: Some(phase.to_string()),
            ..StepState::default()
        },
    );
    state
}

#[test]
fn a_flow_waiting_on_its_merge_approval_holds_no_slot() {
    assert!(waits_on_merge(&parked(FlowStatus::WaitingHuman, "Merge")));
}

#[test]
fn a_human_gate_outside_the_merge_phase_holds_no_slot() {
    assert!(waits_on_merge(&parked(FlowStatus::WaitingHuman, "Revue")));
}

#[test]
fn a_paused_flow_holds_no_slot() {
    assert!(waits_on_merge(&parked(FlowStatus::Paused, "Code")));
}

#[test]
fn a_terminal_flow_holds_no_slot() {
    assert!(waits_on_merge(&parked(FlowStatus::Aborted, "Code")));
}

#[test]
fn a_pausing_or_external_wait_keeps_its_slot() {
    assert!(!waits_on_merge(&parked(FlowStatus::Pausing, "Code")));
    assert!(!waits_on_merge(&parked(
        FlowStatus::WaitingExternal,
        "Code"
    )));
}

#[test]
fn a_running_merge_step_keeps_its_slot() {
    assert!(!waits_on_merge(&parked(FlowStatus::Running, "Merge")));
}

#[test]
fn a_running_flow_without_a_current_step_keeps_its_slot() {
    let mut state = parked(FlowStatus::Running, "Merge");
    state.current = None;
    assert!(!waits_on_merge(&state));
}
