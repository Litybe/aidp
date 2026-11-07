use crate::flows::registry::{Registry, WaitProbe};
use crate::models::constants::flow_probes::*;
use crate::models::enums::BuiltinProbe;
use crate::models::{PrStatusSource, ProbeContext, ProbeOutcome};
use aidp_integrations::pr::ReviewDecision;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const KEY: &str = "PIXEL-1";
const MINUTE_MS: i64 = 60 * 1000;
const START_MS: i64 = 1_000_000;
const CURRENT_SHA: &str = "head-current";
const OLD_SHA: &str = "head-old";
const STATUS_DOC_REVIEW: &str = "review";
const STATUS_DOC_THREADS: &str = "threads";
const STATUS_DOC_GATE: &str = "gate";

/// Serves a status document the test can change between polls.
struct FakeSource {
    document: Mutex<Value>,
}

impl FakeSource {
    fn new(document: Value) -> Arc<Self> {
        Arc::new(FakeSource {
            document: Mutex::new(document),
        })
    }

    fn set(&self, document: Value) {
        *self.document.lock().unwrap() = document;
    }
}

impl PrStatusSource for FakeSource {
    fn status(&self, _key: &str) -> Option<Value> {
        Some(self.document.lock().unwrap().clone())
    }
}

fn context(source: &Arc<FakeSource>, now_ms: i64, last_state: Option<Value>) -> ProbeContext {
    ProbeContext {
        key: KEY.to_string(),
        now_ms,
        started_ms: START_MS,
        last_state,
        pr_status: source.clone(),
    }
}

fn poll(probe: &dyn WaitProbe, context: &ProbeContext) -> ProbeOutcome {
    probe.poll(context, &BTreeMap::new())
}

fn document(unresolved: u64, decision: ReviewDecision, gate_ok: bool, reviewers: Value) -> Value {
    json!({
        STATUS_DOC_THREADS: { KEY_UNRESOLVED: unresolved, KEY_ACTIONABLE: unresolved },
        STATUS_DOC_REVIEW: { KEY_DECISION: decision, KEY_REVIEWERS: reviewers },
        STATUS_DOC_GATE: { KEY_OK: gate_ok },
    })
}

#[test]
fn flow_probes_registry_holds_all_pr_probes() {
    let registry = Registry::with_builtins();
    for probe in [BuiltinProbe::PrReviews, BuiltinProbe::PrThreads] {
        assert!(registry.probe(probe.as_str()).is_some());
    }
}

#[test]
fn flow_probes_pr_reviews_two_comments_give_one_satisfaction_after_quiet_period() {
    let probe = super::pr_reviews::PrReviewsProbe;
    let source = FakeSource::new(document(
        1,
        ReviewDecision::ReviewRequired,
        false,
        json!([]),
    ));
    let first_at = START_MS;
    let first = poll(&probe, &context(&source, first_at, None));
    assert!(!first.satisfied);

    source.set(document(
        2,
        ReviewDecision::ReviewRequired,
        false,
        json!([]),
    ));
    let second_at = first_at + 3 * MINUTE_MS;
    let second = poll(&probe, &context(&source, second_at, first.state));
    assert!(!second.satisfied);

    let ten_after_first = first_at + REVIEWS_QUIET_MS;
    let early = poll(
        &probe,
        &context(&source, ten_after_first, second.state.clone()),
    );
    assert!(!early.satisfied);

    let ten_after_second = second_at + REVIEWS_QUIET_MS;
    let done = poll(&probe, &context(&source, ten_after_second, early.state));
    assert!(done.satisfied);
    assert_eq!(done.outputs[OUTPUT_NEW_THREADS], json!(2));
    assert_eq!(done.outputs[OUTPUT_APPROVED], json!(false));
}

#[test]
fn flow_probes_pr_reviews_without_activity_never_satisfies() {
    let probe = super::pr_reviews::PrReviewsProbe;
    let source = FakeSource::new(document(
        0,
        ReviewDecision::ReviewRequired,
        false,
        json!([]),
    ));
    let outcome = poll(&probe, &context(&source, START_MS + 60 * MINUTE_MS, None));
    assert!(!outcome.satisfied);
}

#[test]
fn flow_probes_pr_reviews_approved_and_green_ends_at_once() {
    let probe = super::pr_reviews::PrReviewsProbe;
    let source = FakeSource::new(document(0, ReviewDecision::Approved, true, json!([])));
    let outcome = poll(&probe, &context(&source, START_MS, None));
    assert!(outcome.satisfied);
    assert_eq!(outcome.outputs[OUTPUT_APPROVED], json!(true));
    assert_eq!(outcome.outputs[OUTPUT_NEW_THREADS], json!(0));
}

#[test]
fn flow_probes_pr_reviews_approved_with_red_gate_keeps_waiting() {
    let probe = super::pr_reviews::PrReviewsProbe;
    let source = FakeSource::new(document(0, ReviewDecision::Approved, false, json!([])));
    let outcome = poll(&probe, &context(&source, START_MS, None));
    assert!(!outcome.satisfied);
}

/// Fresh thread snapshots gate all authors and report a clean PR after handling.
#[test]
fn flow_probes_thread_snapshot_tracks_current_actionable_threads() {
    let source = FakeSource::new(document(
        2,
        ReviewDecision::ReviewRequired,
        false,
        json!([]),
    ));
    let probe = super::pr_threads::PrThreadsProbe;
    let first = poll(&probe, &context(&source, START_MS, None));
    assert_eq!(first.outputs[OUTPUT_THREADS], json!(2));
    assert_eq!(first.outputs[OUTPUT_CLEAN], json!(false));
    source.set(document(
        0,
        ReviewDecision::ReviewRequired,
        false,
        json!([]),
    ));
    let handled = poll(&probe, &context(&source, START_MS, None));
    assert_eq!(handled.outputs[OUTPUT_THREADS], json!(0));
    assert_eq!(handled.outputs[OUTPUT_CLEAN], json!(true));
}

/// Replies handled during the quiet period are not reported as new work.
#[test]
fn flow_probes_handled_threads_do_not_reappear_after_quiet_period() {
    let source = FakeSource::new(document(
        2,
        ReviewDecision::ReviewRequired,
        false,
        json!([]),
    ));
    let probe = super::pr_reviews::PrReviewsProbe;
    let first = poll(&probe, &context(&source, START_MS, None));
    source.set(document(
        0,
        ReviewDecision::ReviewRequired,
        false,
        json!([]),
    ));
    let handled = poll(
        &probe,
        &context(&source, START_MS + REVIEWS_QUIET_MS, first.state),
    );
    assert!(!handled.satisfied);
    let later = poll(
        &probe,
        &context(&source, START_MS + 2 * REVIEWS_QUIET_MS, handled.state),
    );
    assert!(!later.satisfied);
}

/// A same-size new batch restarts the quiet period using comment identities.
#[test]
fn flow_probes_new_reply_identity_restarts_the_quiet_period() {
    let mut status = document(1, ReviewDecision::ReviewRequired, false, json!([]));
    status[KEY_THREADS][KEY_ACTIONABLE_IDS] = json!([OLD_SHA]);
    let source = FakeSource::new(status.clone());
    let probe = super::pr_reviews::PrReviewsProbe;
    let first = poll(&probe, &context(&source, START_MS, None));
    status[KEY_THREADS][KEY_ACTIONABLE_IDS] = json!([CURRENT_SHA]);
    source.set(status);
    let changed = poll(
        &probe,
        &context(&source, START_MS + REVIEWS_QUIET_MS, first.state),
    );
    assert!(!changed.satisfied);
    let quiet = poll(
        &probe,
        &context(&source, START_MS + 2 * REVIEWS_QUIET_MS, changed.state),
    );
    assert!(quiet.satisfied);
    assert_eq!(quiet.outputs[OUTPUT_NEW_THREADS], json!(1));
}
