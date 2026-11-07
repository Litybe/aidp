use crate::flows::registry::WaitProbe;
use crate::models::constants::flow_probes::*;
use crate::models::{ActivityState, ProbeContext, ProbeOutcome};
use aidp_integrations::pr::ReviewDecision;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Satisfied once review activity went quiet, or at once when approved with a green gate.
pub struct PrReviewsProbe;

impl WaitProbe for PrReviewsProbe {
    fn poll(&self, context: &ProbeContext, _with: &BTreeMap<String, Value>) -> ProbeOutcome {
        let Some(status) = context.pr_status.refresh(&context.key) else {
            return ProbeOutcome::pending(context.last_state.clone());
        };
        let Some(unresolved) = super::pr_threads::actionable_count(&status) else {
            return ProbeOutcome::pending(context.last_state.clone());
        };
        let ids = status
            .get(KEY_THREADS)
            .and_then(|threads| threads.get(KEY_ACTIONABLE_IDS))
            .and_then(|ids| serde_json::from_value::<Vec<String>>(ids.clone()).ok())
            .unwrap_or_default();
        let mut state = ActivityState::from_last(context.last_state.as_ref());
        if unresolved != state.seen || ids != state.thread_ids {
            state.last_event_ms = context.now_ms;
        }
        state.seen = unresolved;
        state.thread_ids = ids;
        state.pending_new = unresolved;
        if unresolved == 0 && approved_and_green(&status) {
            return ProbeOutcome::done(outputs(0, true));
        }
        let quiet = context.now_ms - state.last_event_ms >= REVIEWS_QUIET_MS;
        if state.pending_new > 0 && quiet {
            return ProbeOutcome::done(outputs(state.pending_new, false));
        }
        ProbeOutcome::pending(Some(state.to_value()))
    }
}

fn approved_and_green(status: &Value) -> bool {
    let approved = status
        .get(KEY_REVIEW)
        .and_then(|review| {
            review
                .get(KEY_DECISION)
                .and_then(|decision| {
                    serde_json::from_value::<ReviewDecision>(decision.clone()).ok()
                })
                .map(|decision| decision == ReviewDecision::Approved)
        })
        .unwrap_or(false);
    let green = status
        .get(crate::models::constants::pr_status_keys::KEY_GATE)
        .and_then(|gate| gate.get(KEY_OK).and_then(Value::as_bool))
        .unwrap_or(false);
    approved && green
}

fn outputs(new_threads: u64, approved: bool) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (OUTPUT_NEW_THREADS.to_string(), json!(new_threads)),
        (OUTPUT_APPROVED.to_string(), json!(approved)),
    ])
}
