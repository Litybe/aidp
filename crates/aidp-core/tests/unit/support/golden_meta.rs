use serde::Serialize;
use serde_json::Value;

/// The part of an emission other than the script that the snapshots pin.
#[derive(Serialize)]
pub(crate) struct GoldenMeta {
    pub(crate) error: Option<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) gates: Vec<String>,
    pub(crate) awaiting_plan: Option<Value>,
    pub(crate) awaiting_composite: Option<Value>,
    pub(crate) plan_schedules: Value,
}
