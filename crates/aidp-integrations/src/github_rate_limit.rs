//! The GitHub quotas of the authenticated user, read through `gh api rate_limit`.

use crate::models::constants::github_rate_limit::{
    GH_RATE_LIMIT_PATH, KEY_LIMIT, KEY_REMAINING, KEY_RESET, KEY_USED, MESSAGE_NO_RATE_LIMIT,
    PTR_CORE_BUCKET, PTR_GRAPHQL_BUCKET,
};
use crate::models::constants::squad::GH_API;
use crate::models::github_rate_bucket::GithubRateBucket;
use crate::models::github_rate_limit::GithubRateLimit;
use crate::squad::GhRunner;
use aidp_core::clock::Clock;
use chrono::{DateTime, SecondsFormat};
use serde_json::Value;

fn bucket(response: &Value, pointer: &str) -> Option<GithubRateBucket> {
    let node = response.pointer(pointer)?;
    let number = |key: &str| node.get(key).and_then(Value::as_u64);
    let reset = number(KEY_RESET).and_then(|seconds| i64::try_from(seconds).ok())?;
    Some(GithubRateBucket {
        limit: number(KEY_LIMIT)?,
        remaining: number(KEY_REMAINING)?,
        used: number(KEY_USED).unwrap_or_default(),
        reset_at: DateTime::from_timestamp(reset, 0)?.to_rfc3339_opts(SecondsFormat::Secs, true),
    })
}

/// The GraphQL and REST quotas; the call itself costs nothing against them.
pub fn rate_limit(runner: &dyn GhRunner, clock: &dyn Clock) -> Result<GithubRateLimit, String> {
    let args = [GH_API, GH_RATE_LIMIT_PATH].map(str::to_string);
    let response: Value =
        serde_json::from_str(&runner.run(&args)?).map_err(|_| MESSAGE_NO_RATE_LIMIT.to_string())?;
    Ok(GithubRateLimit {
        graphql: bucket(&response, PTR_GRAPHQL_BUCKET),
        core: bucket(&response, PTR_CORE_BUCKET),
        fetched_at: clock.now_rfc3339(),
    })
}

#[cfg(test)]
#[path = "../tests/unit/github_rate_limit_tests.rs"]
mod tests;
