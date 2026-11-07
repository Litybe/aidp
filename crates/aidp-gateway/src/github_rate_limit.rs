//! The GitHub quotas served to the viewer, cached briefly: reading them costs no quota
//! but spawns `gh`.

use crate::sync::lock;
use aidp_core::clock::{Clock, SystemClock};
use aidp_integrations::github_rate_limit::rate_limit;
use aidp_integrations::models::github_rate_limit::GithubRateLimit;
use aidp_integrations::squad::OsGhRunner;
use std::sync::Mutex;

pub const RATE_LIMIT_TTL_MS: i64 = 30_000;

/// Source of the quotas; replaceable in tests.
pub trait RateLimitReader: Send + Sync {
    fn read(&self) -> Result<GithubRateLimit, String>;
}

/// The real reader: `gh api rate_limit`.
pub struct GhRateLimitReader;

impl RateLimitReader for GhRateLimitReader {
    fn read(&self) -> Result<GithubRateLimit, String> {
        rate_limit(&OsGhRunner, &SystemClock)
    }
}

/// The last quotas read, valid [`RATE_LIMIT_TTL_MS`].
pub struct RateLimitCache {
    clock: Box<dyn Clock>,
    reader: Box<dyn RateLimitReader>,
    entry: Mutex<Option<(GithubRateLimit, i64)>>,
}

impl RateLimitCache {
    pub fn new(clock: Box<dyn Clock>, reader: Box<dyn RateLimitReader>) -> Self {
        RateLimitCache {
            clock,
            reader,
            entry: Mutex::new(None),
        }
    }

    /// The cached quotas while fresh, unless `refresh` asks for a new read.
    pub fn get(&self, refresh: bool) -> Result<GithubRateLimit, String> {
        let now = self.clock.now_ms();
        if !refresh {
            if let Some((limits, read_at)) = lock(&self.entry).clone() {
                if now.saturating_sub(read_at) < RATE_LIMIT_TTL_MS {
                    return Ok(limits);
                }
            }
        }
        let limits = self.reader.read()?;
        *lock(&self.entry) = Some((limits.clone(), now));
        Ok(limits)
    }
}

#[cfg(test)]
#[path = "../tests/unit/github_rate_limit_tests.rs"]
mod tests;
