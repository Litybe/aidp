use crate::models::enums::FlowEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// What the executor must do. The machine never performs any of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EffectKind {
    RunAction {
        step: String,
        name: String,
        with: BTreeMap<String, Value>,
    },
    StartPipeline {
        step: String,
        template: String,
        with: BTreeMap<String, Value>,
    },
    CancelSession {
        step: String,
    },
    CreateInboxEntry {
        step: String,
        prompt: Option<String>,
        choices: Vec<String>,
    },
    ResolveInboxEntry {
        step: String,
    },
    StartWait {
        step: String,
        probe: String,
        with: BTreeMap<String, Value>,
    },
    Journal {
        at: String,
        line: String,
    },
    Emit {
        event: FlowEvent,
        step: Option<String>,
        detail: Option<String>,
    },
}
