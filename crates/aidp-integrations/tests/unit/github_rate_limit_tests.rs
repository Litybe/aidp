use super::rate_limit;
use crate::squad::test_support::FakeRunner;
use aidp_core::clock::FixedClock;

const RATE_LIMIT: &str = include_str!("../fixtures/github_rate_limit/rate_limit.json");
const NOW: &str = "2026-10-09T08:00:00Z";
const GRAPHQL_RESET: &str = "2026-10-09T09:30:00Z";
const GRAPHQL_REMAINING: u64 = 3760;
const GRAPHQL_USED: u64 = 1240;
const CORE_REMAINING: u64 = 4988;
const LIMIT: u64 = 5000;
const GH_RATE_LIMIT_ARGS: [&str; 2] = ["api", "rate_limit"];
const GH_FAILURE: &str = "gh: not logged in";
const NOT_JSON: &str = "not json";
const WITHOUT_GRAPHQL: &str =
    r#"{"resources":{"core":{"limit":5000,"used":0,"remaining":5000,"reset":1791540000}}}"#;

#[test]
fn reads_the_graphql_and_rest_quotas_with_their_reset_instant() {
    let runner = FakeRunner::new(Box::new(|_| Ok(RATE_LIMIT.to_string())));
    let limits = rate_limit(&runner, &FixedClock::from_rfc3339(NOW)).unwrap();
    let graphql = limits.graphql.unwrap();
    assert_eq!(graphql.limit, LIMIT);
    assert_eq!(graphql.remaining, GRAPHQL_REMAINING);
    assert_eq!(graphql.used, GRAPHQL_USED);
    assert_eq!(graphql.reset_at, GRAPHQL_RESET);
    assert_eq!(limits.core.unwrap().remaining, CORE_REMAINING);
    assert_eq!(runner.calls()[0], GH_RATE_LIMIT_ARGS.map(str::to_string));
}

#[test]
fn a_quota_github_leaves_out_is_absent() {
    let runner = FakeRunner::new(Box::new(|_| Ok(WITHOUT_GRAPHQL.to_string())));
    let limits = rate_limit(&runner, &FixedClock::from_rfc3339(NOW)).unwrap();
    assert!(limits.graphql.is_none());
    assert!(limits.core.is_some());
}

#[test]
fn a_failed_or_unreadable_call_is_an_error() {
    let failing = FakeRunner::new(Box::new(|_| Err(GH_FAILURE.to_string())));
    assert!(rate_limit(&failing, &FixedClock::from_rfc3339(NOW)).is_err());
    let garbled = FakeRunner::new(Box::new(|_| Ok(NOT_JSON.to_string())));
    assert!(rate_limit(&garbled, &FixedClock::from_rfc3339(NOW)).is_err());
}
