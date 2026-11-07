use crate::models::enums::{ActionOutcome, ManualRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Inputs of the state machine. Every event carries its own timestamp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Start {
        ticket: Value,
        at: String,
    },
    ActionFinished {
        effect_id: String,
        outcome: ActionOutcome,
        at: String,
    },
    PipelineFinished {
        effect_id: String,
        run_status: String,
        outputs: BTreeMap<String, Value>,
        at: String,
    },
    PipelineOrphaned {
        effect_id: String,
        at: String,
    },
    BudgetInsufficient {
        step: String,
        at: String,
    },
    /// The resource a pipeline step needs is held by another run.
    ResourceBusy {
        step: String,
        resource: String,
        at: String,
    },
    /// The ticket reached its token ceiling: the pipeline step waits for a "continue?" decision.
    TicketBudgetExceeded {
        step: String,
        detail: String,
        at: String,
    },
    HumanDecision {
        step: String,
        decision: String,
        actor: String,
        at: String,
        report: Option<String>,
    },
    WaitSatisfied {
        effect_id: String,
        #[serde(default)]
        outputs: BTreeMap<String, Value>,
        at: String,
    },
    Pause {
        at: String,
    },
    Resume {
        at: String,
    },
    Abort {
        at: String,
    },
    Retry {
        step: String,
        at: String,
    },
    Skip {
        step: String,
        at: String,
    },
    Manual {
        request: ManualRequest,
        at: String,
    },
}

impl Event {
    /// The timestamp carried by the event.
    pub fn at(&self) -> &str {
        match self {
            Event::Start { at, .. }
            | Event::ActionFinished { at, .. }
            | Event::PipelineFinished { at, .. }
            | Event::PipelineOrphaned { at, .. }
            | Event::BudgetInsufficient { at, .. }
            | Event::ResourceBusy { at, .. }
            | Event::TicketBudgetExceeded { at, .. }
            | Event::HumanDecision { at, .. }
            | Event::WaitSatisfied { at, .. }
            | Event::Pause { at }
            | Event::Resume { at }
            | Event::Abort { at }
            | Event::Retry { at, .. }
            | Event::Skip { at, .. }
            | Event::Manual { at, .. } => at,
        }
    }
}
