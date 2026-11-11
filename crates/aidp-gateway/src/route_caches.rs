//! The stale-while-revalidate caches of the slow read routes, held by the `App`.

use crate::models::constants::swr_cache::{MEMORY_TTL, WORKTREES_TTL};
use crate::swr_cache::SwrCache;
use serde_json::Value;

/// A status and a JSON body, as the memory routes answer.
pub type CachedReply = (u16, Value);

/// One cache per route that shells out or opens a store; each keyed by its query.
pub struct RouteCaches {
    pub worktrees: SwrCache<String, Value>,
    pub memory: SwrCache<String, CachedReply>,
}

impl Default for RouteCaches {
    fn default() -> Self {
        RouteCaches {
            worktrees: SwrCache::new(WORKTREES_TTL),
            memory: SwrCache::new(MEMORY_TTL),
        }
    }
}
