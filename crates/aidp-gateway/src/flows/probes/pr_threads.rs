use crate::flows::registry::WaitProbe;
use crate::models::constants::flow_probes::*;
use crate::models::{ProbeContext, ProbeOutcome};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Reads actionable review threads immediately; unreadable snapshots remain pending.
pub struct PrThreadsProbe;

impl WaitProbe for PrThreadsProbe {
    fn poll(&self, context: &ProbeContext, _with: &BTreeMap<String, Value>) -> ProbeOutcome {
        let Some(threads) = context
            .pr_status
            .refresh(&context.key)
            .as_ref()
            .and_then(actionable_count)
        else {
            return ProbeOutcome::pending(None);
        };
        ProbeOutcome::done(BTreeMap::from([
            (OUTPUT_THREADS.to_string(), json!(threads)),
            (OUTPUT_CLEAN.to_string(), json!(threads == 0)),
        ]))
    }
}

pub(super) fn actionable_count(status: &Value) -> Option<u64> {
    status.get(KEY_THREADS)?.get(KEY_ACTIONABLE)?.as_u64()
}
