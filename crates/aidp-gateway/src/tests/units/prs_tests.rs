//! Cache behaviour, invalidation and webhook target extraction.

use super::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

const REPO: &str = "acme/svc";
const NUMBER: u64 = 7;
const WITHIN_TTL_MS: i64 = 19_000;
const BEYOND_TTL_MS: i64 = 21_000;
const START_MS: i64 = 1_000;
const BASE: &str = "master";
const STATUS_SHA: &str = "abc123";
const OTHER_SHA: &str = "def456";
const OTHER_NUMBER: u64 = 8;

struct TickingClock(Arc<AtomicI64>);

impl Clock for TickingClock {
    fn now_utc(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(self.0.load(Ordering::SeqCst)).unwrap()
    }
    fn now_rfc3339(&self) -> String {
        self.now_utc().to_rfc3339()
    }
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn cache_with_time() -> (PrStatusCache, Arc<AtomicI64>) {
    let time = Arc::new(AtomicI64::new(START_MS));
    (
        PrStatusCache::new(Box::new(TickingClock(time.clone()))),
        time,
    )
}

fn fetch_counted(cache: &PrStatusCache, calls: &Cell<u32>, refresh: bool) -> Value {
    cache
        .get(REPO, NUMBER, refresh, || {
            calls.set(calls.get() + 1);
            Ok(json!({ "n": calls.get() }))
        })
        .unwrap()
}

#[test]
fn two_reads_within_ttl_fetch_once() {
    let (cache, time) = cache_with_time();
    let calls = Cell::new(0);
    fetch_counted(&cache, &calls, false);
    time.fetch_add(WITHIN_TTL_MS, Ordering::SeqCst);
    fetch_counted(&cache, &calls, false);
    assert_eq!(calls.get(), 1);
}

#[test]
fn refresh_bypasses_the_cache() {
    let (cache, _) = cache_with_time();
    let calls = Cell::new(0);
    fetch_counted(&cache, &calls, false);
    let value = fetch_counted(&cache, &calls, true);
    assert_eq!(calls.get(), 2);
    assert_eq!(value["n"], 2);
}

#[test]
fn expired_entry_is_refetched() {
    let (cache, time) = cache_with_time();
    let calls = Cell::new(0);
    fetch_counted(&cache, &calls, false);
    time.fetch_add(BEYOND_TTL_MS, Ordering::SeqCst);
    fetch_counted(&cache, &calls, false);
    assert_eq!(calls.get(), 2);
}

#[test]
fn failed_fetch_is_not_cached() {
    let (cache, _) = cache_with_time();
    let failure = cache.get(REPO, NUMBER, false, || {
        Err(IntegrationError::Upstream("down".to_string()))
    });
    assert!(failure.is_err());
    assert_eq!(cache.cached_count(), 0);
}

#[test]
fn invalidate_empties_and_emits_event() {
    let (cache, _) = cache_with_time();
    let bus = Bus::new();
    let calls = Cell::new(0);
    fetch_counted(&cache, &calls, false);
    cache.invalidate(&bus, REPO, NUMBER);
    assert_eq!(cache.cached_count(), 0);
    let events = bus.recent(10, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], PR_STATUS_EVENT);
    assert_eq!(events[0]["data"], json!({ "repo": REPO, "number": NUMBER }));
}

#[test]
fn rules_are_cached_for_an_hour() {
    let (cache, time) = cache_with_time();
    let calls = Cell::new(0);
    let fetch = || {
        calls.set(calls.get() + 1);
        BranchRules::default()
    };
    cache.rules(REPO, BASE, fetch);
    time.fetch_add(RULES_TTL_MS - 1, Ordering::SeqCst);
    cache.rules(REPO, BASE, fetch);
    assert_eq!(calls.get(), 1);
    time.fetch_add(2, Ordering::SeqCst);
    cache.rules(REPO, BASE, fetch);
    assert_eq!(calls.get(), 2);
}

/// A degraded rules read is not stored, so the next call fetches again.
#[test]
fn degraded_rules_are_refetched_on_the_next_call() {
    let (cache, _) = cache_with_time();
    let calls = Cell::new(0);
    let fetch = || {
        calls.set(calls.get() + 1);
        BranchRules {
            degraded: true,
            ..BranchRules::default()
        }
    };
    cache.rules(REPO, BASE, fetch);
    cache.rules(REPO, BASE, fetch);
    assert_eq!(calls.get(), 2);
}

/// Only the cached PRs whose head sha matches are dropped, each emitting an event.
#[test]
fn invalidate_by_sha_drops_only_matching_heads() {
    let (cache, _) = cache_with_time();
    let bus = Bus::new();
    cache
        .get(REPO, NUMBER, false, || {
            Ok(json!({ "head": { "sha": STATUS_SHA } }))
        })
        .unwrap();
    cache
        .get(REPO, OTHER_NUMBER, false, || {
            Ok(json!({ "head": { "sha": OTHER_SHA } }))
        })
        .unwrap();
    cache.invalidate_by_sha(&bus, REPO, STATUS_SHA);
    assert_eq!(cache.cached_count(), 1);
    let events = bus.recent(10, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"], json!({ "repo": REPO, "number": NUMBER }));
}

/// A status delivery yields its repository and commit sha.
#[test]
fn it_status_event_yields_repo_and_sha() {
    assert_eq!(
        status_target_from_webhook("status", &fixture("status")),
        Some((REPO.to_string(), STATUS_SHA.to_string()))
    );
    assert_eq!(
        status_target_from_webhook("check_run", &fixture("check_run")),
        None
    );
}

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/src/tests/units/fixtures/prs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn targets(event: &str) -> Vec<(String, u64)> {
    pr_targets_from_webhook(event, &fixture(event))
}

#[test]
fn it_pull_request_events_yield_their_number() {
    assert_eq!(targets("pull_request"), vec![(REPO.to_string(), 7)]);
    assert_eq!(targets("pull_request_review"), vec![(REPO.to_string(), 8)]);
    assert_eq!(
        targets("pull_request_review_comment"),
        vec![(REPO.to_string(), 9)]
    );
}

#[test]
fn it_check_events_yield_every_attached_pull_request() {
    assert_eq!(
        targets("check_run"),
        vec![(REPO.to_string(), 10), (REPO.to_string(), 11)]
    );
    assert_eq!(targets("check_suite"), vec![(REPO.to_string(), 12)]);
}

#[test]
fn it_status_event_yields_no_target() {
    assert!(targets("status").is_empty());
}

#[test]
fn no_pr_error_carries_a_stable_reason() {
    let error = no_pr_error("feat/LOC-1");
    assert_eq!(error.status(), 404);
    assert_eq!(error.body()["reason"], "no-pull-request");
    assert_eq!(error.error.message, "no pull request for feat/LOC-1");
}

#[test]
fn pr_url_number_is_parsed() {
    assert_eq!(
        number_from_pr_url("https://github.com/a/b/pull/42"),
        Some(42)
    );
    assert_eq!(number_from_pr_url("https://github.com/a/b/pull/x"), None);
}
