use serde_json::Value;
use std::collections::BTreeMap;

/// What one poll of a wait probe concluded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProbeOutcome {
    /// True once the awaited fact holds.
    pub satisfied: bool,
    /// Step outputs, delivered when the probe is satisfied.
    pub outputs: BTreeMap<String, Value>,
    /// Probe memory kept in the wait record for the next poll.
    pub state: Option<Value>,
}

impl ProbeOutcome {
    /// Not satisfied yet, remembering `state` for the next poll.
    pub fn pending(state: Option<Value>) -> Self {
        ProbeOutcome {
            satisfied: false,
            outputs: BTreeMap::new(),
            state,
        }
    }

    /// Satisfied, delivering `outputs`.
    pub fn done(outputs: BTreeMap<String, Value>) -> Self {
        ProbeOutcome {
            satisfied: true,
            outputs,
            state: None,
        }
    }
}
