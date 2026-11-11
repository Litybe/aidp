//! SQLite cache of the vault run index. The run files stay the source of truth (the `aidp` CLI
//! writes them from other processes); this database only remembers what the daemon read, so a
//! restart loads the index instead of walking the vault. Only this process writes to it.

use crate::models::constants::vault_index::{
    BUSY_TIMEOUT_MS, DB_SIDE_FILE_SUFFIXES, DROP_SQL, SCHEMA_SQL, SCHEMA_VERSION,
};
use crate::models::stored_run::StoredRun;
use crate::pixellab_usage::{BalanceSnapshot, RunPixellabUsage};
use crate::sync::lock;
use crate::ticket_cost::{StageRow, TokenTotals};
use crate::vault_index::{RunSignature, RunSummary};
use anyhow::Result;
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USER_VERSION: &str = "user_version";
const SELECT_RUNS: &str = "SELECT run_dir, ticket, repo, run_id, route, state_len, \
    state_mtime_ns, dir_mtime_ns, pl_generations, pl_jobs, pl_balance_before, pl_balance_after, \
    pl_spent, pl_recorded_at, pl_stages, balance_amount, balance_raw, balance_recorded_at \
    FROM runs WHERE vault = ?1";
const SELECT_STAGES: &str = "SELECT stages.run_dir, stages.run_id, stages.stage, stages.model, \
    stages.input, stages.output, stages.cache_read, stages.cache_create \
    FROM stages JOIN runs ON runs.run_dir = stages.run_dir WHERE runs.vault = ?1 \
    ORDER BY stages.run_dir, stages.position";
const INSERT_RUN: &str = "INSERT OR REPLACE INTO runs (run_dir, vault, repo, ticket, run_id, \
    route, state_len, state_mtime_ns, dir_mtime_ns, pl_generations, pl_jobs, pl_balance_before, \
    pl_balance_after, pl_spent, pl_recorded_at, pl_stages, balance_amount, balance_raw, \
    balance_recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
    ?15, ?16, ?17, ?18, ?19)";
const INSERT_STAGE: &str = "INSERT INTO stages (run_dir, position, run_id, stage, model, input, \
    output, cache_read, cache_create) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";
const DELETE_STAGES_OF_RUN: &str = "DELETE FROM stages WHERE run_dir = ?1";
const DELETE_RUN: &str = "DELETE FROM runs WHERE run_dir = ?1";
const DELETE_STAGES_OF_VAULT: &str =
    "DELETE FROM stages WHERE run_dir IN (SELECT run_dir FROM runs WHERE vault = ?1)";
const DELETE_RUNS_OF_VAULT: &str = "DELETE FROM runs WHERE vault = ?1";
const MARK_SCANNED: &str = "INSERT OR IGNORE INTO vaults (vault) VALUES (?1)";
const UNMARK_SCANNED: &str = "DELETE FROM vaults WHERE vault = ?1";
const IS_SCANNED: &str = "SELECT COUNT(*) FROM vaults WHERE vault = ?1";

/// Handle on the cache database; every call goes through its single connection.
pub struct IndexStore {
    connection: Mutex<Connection>,
}

fn text_of(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn nanos_of(time: Option<SystemTime>) -> Option<i64> {
    let elapsed = time?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(elapsed.as_nanos()).ok()
}

fn time_of(nanos: Option<i64>) -> Option<SystemTime> {
    let nanos = u64::try_from(nanos?).ok()?;
    Some(UNIX_EPOCH + Duration::from_nanos(nanos))
}

fn stored_count(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn count_of(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn small_count_of(value: i64) -> u32 {
    u32::try_from(count_of(value)).unwrap_or(u32::MAX)
}

impl IndexStore {
    /// Opens the database at `path`, creating it; another schema version drops and recreates it.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let version: u32 = connection.pragma_query_value(None, USER_VERSION, |row| row.get(0))?;
        if version == SCHEMA_VERSION {
            connection.execute_batch(SCHEMA_SQL)?;
        } else {
            connection.execute_batch(&format!(
                "BEGIN; {DROP_SQL} {SCHEMA_SQL} PRAGMA {USER_VERSION} = {SCHEMA_VERSION}; COMMIT;"
            ))?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Opens the database; a file SQLite cannot use is deleted and created again (it is a cache).
    pub fn open_or_recreate(path: &Path) -> Result<Self> {
        match Self::open(path) {
            Ok(store) => Ok(store),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "vault index cache unusable, recreating it");
                let _ = std::fs::remove_file(path);
                for suffix in DB_SIDE_FILE_SUFFIXES {
                    let mut side = path.as_os_str().to_os_string();
                    side.push(suffix);
                    let _ = std::fs::remove_file(PathBuf::from(side));
                }
                Self::open(path)
            }
        }
    }

    /// Whether a full scan of `vault` was stored: only then is [`Self::load`] a complete index.
    pub fn is_scanned(&self, vault: &Path) -> Result<bool> {
        let count: i64 =
            lock(&self.connection)
                .query_row(IS_SCANNED, params![text_of(vault)], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Every run stored for `vault`, with its stage rows.
    pub fn load(&self, vault: &Path) -> Result<Vec<StoredRun>> {
        let connection = lock(&self.connection);
        let vault = text_of(vault);
        let mut stages: HashMap<String, Vec<StageRow>> = HashMap::new();
        let mut statement = connection.prepare(SELECT_STAGES)?;
        let mut rows = statement.query(params![vault])?;
        while let Some(row) = rows.next()? {
            let run_dir: String = row.get(0)?;
            stages.entry(run_dir).or_default().push(StageRow {
                run_id: row.get(1)?,
                stage: row.get(2)?,
                model: row.get(3)?,
                tokens: TokenTotals {
                    input: count_of(row.get(4)?),
                    output: count_of(row.get(5)?),
                    cache_read: count_of(row.get(6)?),
                    cache_create: count_of(row.get(7)?),
                },
            });
        }
        let mut statement = connection.prepare(SELECT_RUNS)?;
        let mut rows = statement.query(params![vault])?;
        let mut runs = Vec::new();
        while let Some(row) = rows.next()? {
            let run_dir: String = row.get(0)?;
            let run_id: String = row.get(3)?;
            let recorded_at: Option<String> = row.get(17)?;
            let raw: Option<String> = row.get(16)?;
            let balance = match raw {
                Some(raw) => Some(BalanceSnapshot {
                    amount: row.get(15)?,
                    raw: serde_json::from_str(&raw)?,
                    recorded_at: recorded_at.unwrap_or_default(),
                    run_id: run_id.clone(),
                }),
                None => None,
            };
            let summary = RunSummary {
                repo: row.get(2)?,
                route: row.get(4)?,
                stages: stages.remove(&run_dir).unwrap_or_default(),
                pixellab: RunPixellabUsage {
                    run_id: run_id.clone(),
                    generations: small_count_of(row.get(8)?),
                    jobs: small_count_of(row.get(9)?),
                    balance_before: row.get(10)?,
                    balance_after: row.get(11)?,
                    spent: row.get(12)?,
                    recorded_at: row.get(13)?,
                    stages: small_count_of(row.get(14)?),
                },
                balance,
                signature: RunSignature {
                    state_len: count_of(row.get(5)?),
                    state_modified: time_of(row.get(6)?),
                    dir_modified: time_of(row.get(7)?),
                },
                run_id,
            };
            runs.push(StoredRun {
                run_dir: PathBuf::from(run_dir),
                ticket: row.get(1)?,
                run: Arc::new(summary),
            });
        }
        Ok(runs)
    }

    /// Replaces everything stored for `vault` by `runs`, the result of a full scan.
    pub fn replace(&self, vault: &Path, runs: &[StoredRun]) -> Result<()> {
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction()?;
        let vault_text = text_of(vault);
        transaction.execute(DELETE_STAGES_OF_VAULT, params![vault_text])?;
        transaction.execute(DELETE_RUNS_OF_VAULT, params![vault_text])?;
        for stored in runs {
            write_run(&transaction, &vault_text, stored)?;
        }
        transaction.execute(MARK_SCANNED, params![vault_text])?;
        transaction.commit()?;
        Ok(())
    }

    /// Writes the runs that changed and drops the runs whose directory vanished, in one
    /// transaction.
    pub fn apply(&self, vault: &Path, upserts: &[StoredRun], removed: &[PathBuf]) -> Result<()> {
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction()?;
        let vault_text = text_of(vault);
        for run_dir in removed {
            let run_dir = text_of(run_dir);
            transaction.execute(DELETE_STAGES_OF_RUN, params![run_dir])?;
            transaction.execute(DELETE_RUN, params![run_dir])?;
        }
        for stored in upserts {
            write_run(&transaction, &vault_text, stored)?;
        }
        transaction.execute(MARK_SCANNED, params![vault_text])?;
        transaction.commit()?;
        Ok(())
    }

    /// Marks `vault` as not scanned, so the next start walks it instead of trusting the rows.
    pub fn forget(&self, vault: &Path) -> Result<()> {
        lock(&self.connection).execute(UNMARK_SCANNED, params![text_of(vault)])?;
        Ok(())
    }
}

fn write_run(transaction: &Transaction<'_>, vault: &str, stored: &StoredRun) -> Result<()> {
    let run = &stored.run;
    let run_dir = text_of(&stored.run_dir);
    transaction.execute(DELETE_STAGES_OF_RUN, params![run_dir])?;
    transaction.execute(
        INSERT_RUN,
        params![
            run_dir,
            vault,
            run.repo,
            stored.ticket,
            run.run_id,
            run.route,
            stored_count(run.signature.state_len),
            nanos_of(run.signature.state_modified),
            nanos_of(run.signature.dir_modified),
            i64::from(run.pixellab.generations),
            i64::from(run.pixellab.jobs),
            run.pixellab.balance_before,
            run.pixellab.balance_after,
            run.pixellab.spent,
            run.pixellab.recorded_at,
            i64::from(run.pixellab.stages),
            run.balance.as_ref().and_then(|balance| balance.amount),
            run.balance.as_ref().map(|balance| balance.raw.to_string()),
            run.balance
                .as_ref()
                .map(|balance| balance.recorded_at.clone()),
        ],
    )?;
    let mut insert = transaction.prepare_cached(INSERT_STAGE)?;
    for (position, row) in run.stages.iter().enumerate() {
        insert.execute(params![
            run_dir,
            i64::try_from(position).unwrap_or(i64::MAX),
            row.run_id,
            row.stage,
            row.model,
            stored_count(row.tokens.input),
            stored_count(row.tokens.output),
            stored_count(row.tokens.cache_read),
            stored_count(row.tokens.cache_create),
        ])?;
    }
    Ok(())
}
