//! Names of the stale-while-revalidate caches.

use std::time::Duration;

/// Name of the background thread refreshing a stale cache entry.
pub const REFRESH_THREAD_NAME: &str = "swr-refresh";
/// Age after which `GET /api/worktrees` refreshes its rows.
pub const WORKTREES_TTL: Duration = Duration::from_secs(20);
/// Age after which `GET /api/tickets/:key/pr` refreshes its answer; not below the PR status TTL.
pub const TICKET_PR_TTL: Duration = Duration::from_secs(30);
/// Age after which a catalogue read is refreshed.
pub const CATALOG_TTL: Duration = Duration::from_secs(30);
/// Age after which a memory listing is refreshed.
pub const MEMORY_TTL: Duration = Duration::from_secs(10);
