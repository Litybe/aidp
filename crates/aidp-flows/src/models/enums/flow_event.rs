use serde::{Deserialize, Serialize};

/// Events the gateway publishes on its bus for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowEvent {
    #[serde(rename = "flow.started")]
    Started,
    #[serde(rename = "flow.step_changed")]
    StepChanged,
    #[serde(rename = "flow.waiting_you")]
    WaitingYou,
    #[serde(rename = "flow.ended")]
    Ended,
}

impl FlowEvent {
    /// The wire name published on the bus.
    pub fn as_wire(self) -> &'static str {
        match self {
            FlowEvent::Started => "flow.started",
            FlowEvent::StepChanged => "flow.step_changed",
            FlowEvent::WaitingYou => "flow.waiting_you",
            FlowEvent::Ended => "flow.ended",
        }
    }
}
