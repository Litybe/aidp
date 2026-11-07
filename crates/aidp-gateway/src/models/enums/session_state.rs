use serde_json::Value;
use std::collections::BTreeMap;

/// What the executor knows of a session it launched.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    Running,
    Ended {
        run_status: String,
        outputs: BTreeMap<String, Value>,
    },
    Gone,
}
