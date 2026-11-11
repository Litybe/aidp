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

/// File name of the SQLite cache of the index, next to the memory database.
pub const INDEX_DB_FILE: &str = "vault-index.sqlite";
/// Version of the cache schema (`PRAGMA user_version`); another one drops and rebuilds the cache.
pub const SCHEMA_VERSION: u32 = 1;
/// Busy timeout of the cache connection, in milliseconds.
pub const BUSY_TIMEOUT_MS: u64 = 5000;
/// Suffixes of the files SQLite keeps beside a database in WAL mode.
pub const DB_SIDE_FILE_SUFFIXES: [&str; 2] = ["-wal", "-shm"];
/// Statements dropping the cache tables.
pub const DROP_SQL: &str = "
    DROP TABLE IF EXISTS stages;
    DROP TABLE IF EXISTS runs;
    DROP TABLE IF EXISTS vaults;
";
/// Statements creating the cache tables: one row per run, one per stage row, one per scanned vault.
pub const SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS vaults (
        vault TEXT PRIMARY KEY
    );
    CREATE TABLE IF NOT EXISTS runs (
        run_dir TEXT PRIMARY KEY,
        vault TEXT NOT NULL,
        repo TEXT NOT NULL,
        ticket TEXT NOT NULL,
        run_id TEXT NOT NULL,
        route TEXT,
        state_len INTEGER NOT NULL,
        state_mtime_ns INTEGER,
        dir_mtime_ns INTEGER,
        pl_generations INTEGER NOT NULL,
        pl_jobs INTEGER NOT NULL,
        pl_balance_before REAL,
        pl_balance_after REAL,
        pl_spent REAL,
        pl_recorded_at TEXT,
        pl_stages INTEGER NOT NULL,
        balance_amount REAL,
        balance_raw TEXT,
        balance_recorded_at TEXT
    );
    CREATE INDEX IF NOT EXISTS runs_by_vault_ticket ON runs (vault, ticket);
    CREATE TABLE IF NOT EXISTS stages (
        run_dir TEXT NOT NULL REFERENCES runs (run_dir) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        run_id TEXT NOT NULL,
        stage TEXT NOT NULL,
        model TEXT,
        input INTEGER NOT NULL,
        output INTEGER NOT NULL,
        cache_read INTEGER NOT NULL,
        cache_create INTEGER NOT NULL,
        PRIMARY KEY (run_dir, position)
    );
    CREATE INDEX IF NOT EXISTS stages_by_stage ON stages (stage);
";
