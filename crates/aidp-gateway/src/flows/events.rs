//! Publishes the run's lifecycle on the bus: `flow.started`,
//! `flow.step_changed`, `flow.waiting_you` and `flow.ended`.

use crate::bus::Bus;
use aidp_flows::{FlowEvent, FlowState};
use serde_json::{json, Value};

pub const FIELD_KEY: &str = "key";
pub const FIELD_FLOW: &str = "flow";
pub const FIELD_VERSION: &str = "version";
pub const FIELD_STATUS: &str = "status";
pub const FIELD_STEP: &str = "step";
pub const FIELD_PHASE: &str = "phase";
pub const FIELD_DETAIL: &str = "detail";
pub const FIELD_CURRENT: &str = "current";

/// The bus payload of a run event, built from the run's state after the transition.
pub fn payload(state: &FlowState, step: Option<&str>, detail: Option<&str>) -> Value {
    let phase = step
        .and_then(|rid| state.steps.get(rid))
        .and_then(|step_state| step_state.phase.clone());
    json!({
        FIELD_KEY: state.key,
        FIELD_FLOW: state.def.id,
        FIELD_VERSION: state.def.version,
        FIELD_STATUS: state.status.as_wire(),
        FIELD_CURRENT: state.current,
        FIELD_STEP: step,
        FIELD_PHASE: phase,
        FIELD_DETAIL: detail,
    })
}

/// Emits `event` for the run on `bus` and returns the emitted bus event.
pub fn publish(
    bus: &Bus,
    event: FlowEvent,
    state: &FlowState,
    step: Option<&str>,
    detail: Option<&str>,
) -> Value {
    bus.emit(event.as_wire(), payload(state, step, detail))
}
