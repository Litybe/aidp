//! Names and limits of the in-memory index of the vault's pipeline runs.

use std::time::Duration;

/// Age after which a read starts an incremental refresh of the index on a background thread.
pub const INDEX_REFRESH_TTL: Duration = Duration::from_secs(15);
/// Upper bound on the worker threads that scan the vault.
pub const MAX_SCAN_WORKERS: usize = 8;
/// Name of the background thread refreshing the index.
pub const REFRESH_THREAD_NAME: &str = "vault-index-refresh";
/// Name of the background thread building the index at startup.
pub const WARM_THREAD_NAME: &str = "vault-index-warm";
