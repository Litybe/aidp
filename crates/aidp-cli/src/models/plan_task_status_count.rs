use std::collections::BTreeMap;

use serde::Serialize;

/// Per-stage task status tally reported by `aidp workflow plan-status`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct PlanTaskStatusCount<'a> {
    pub counts: BTreeMap<String, usize>,
    pub stage: &'a str,
}
