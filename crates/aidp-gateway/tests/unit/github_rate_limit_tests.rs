use super::{RateLimitCache, RateLimitReader, RATE_LIMIT_TTL_MS};
use aidp_core::clock::Clock;
use aidp_integrations::models::github_rate_bucket::GithubRateBucket;
use aidp_integrations::models::github_rate_limit::GithubRateLimit;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::Arc;

const START_MS: i64 = 1_000;
const LIMIT: u64 = 5000;
const REMAINING: u64 = 4200;
const RESET_AT: &str = "2026-10-09T09:30:00Z";
const FETCHED_AT: &str = "2026-10-09T08:00:00Z";
const READ_FAILURE: &str = "gh: not logged in";

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

struct CountingReader {
    reads: Arc<AtomicU32>,
    fails: bool,
}

impl RateLimitReader for CountingReader {
    fn read(&self) -> Result<GithubRateLimit, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fails {
            return Err(READ_FAILURE.to_string());
        }
        Ok(GithubRateLimit {
            graphql: Some(GithubRateBucket {
                limit: LIMIT,
                remaining: REMAINING,
                used: LIMIT - REMAINING,
                reset_at: RESET_AT.to_string(),
            }),
            core: None,
            fetched_at: FETCHED_AT.to_string(),
        })
    }
}

fn cache(fails: bool) -> (RateLimitCache, Arc<AtomicI64>, Arc<AtomicU32>) {
    let time = Arc::new(AtomicI64::new(START_MS));
    let reads = Arc::new(AtomicU32::new(0));
    let reader = CountingReader {
        reads: reads.clone(),
        fails,
    };
    let cache = RateLimitCache::new(Box::new(TickingClock(time.clone())), Box::new(reader));
    (cache, time, reads)
}

#[test]
fn quotas_are_read_once_within_the_ttl() {
    let (cache, time, reads) = cache(false);
    let first = cache.get(false).unwrap();
    time.fetch_add(RATE_LIMIT_TTL_MS - 1, Ordering::SeqCst);
    assert_eq!(cache.get(false).unwrap(), first);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(first.graphql.unwrap().remaining, REMAINING);
}

#[test]
fn quotas_are_read_again_past_the_ttl_or_on_refresh() {
    let (cache, time, reads) = cache(false);
    cache.get(false).unwrap();
    cache.get(true).unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    time.fetch_add(RATE_LIMIT_TTL_MS, Ordering::SeqCst);
    cache.get(false).unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 3);
}

#[test]
fn a_failed_read_is_an_error_and_is_not_cached() {
    let (cache, _, reads) = cache(true);
    assert_eq!(cache.get(false).unwrap_err(), READ_FAILURE);
    assert!(cache.get(false).is_err());
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}
