//! The SQLite database: one writer connection, WAL, versioned migrations.

use crate::error::{MemoryError, Result};
use crate::models::constants::limits::BUSY_TIMEOUT_MS;
use crate::models::constants::paths::{DB_FILE, MEMORY_DIR};
use crate::models::constants::schema::MIGRATIONS;
use crate::models::purge_report::PurgeReport;
use crate::models::store_stats::StoreStats;
use rusqlite::{params, Connection, Transaction};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

const USER_VERSION: &str = "user_version";
const VACUUM: &str = "VACUUM";

/// Path of the database under a gateway state directory.
pub fn db_path(gateway_dir: &Path) -> PathBuf {
    gateway_dir.join(MEMORY_DIR).join(DB_FILE)
}

/// Handle on the memory database; every call goes through its single connection.
pub struct MemoryStore {
    connection: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl MemoryStore {
    /// Opens `<gateway_dir>/memory/memory.db`, creating it and migrating it.
    pub fn open_in(gateway_dir: &Path) -> Result<Self> {
        Self::open(&db_path(gateway_dir))
    }

    /// Opens the database file at `path`, creating its folder.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        Self::prepare(connection, Some(path.to_path_buf()))
    }

    /// An in-memory database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory()?, None)
    }

    fn prepare(connection: Connection, path: Option<PathBuf>) -> Result<Self> {
        connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let store = Self {
            connection: Mutex::new(connection),
            path,
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let mut connection = self.lock();
        let current: usize = connection.pragma_query_value(None, USER_VERSION, |row| row.get(0))?;
        for (index, script) in MIGRATIONS.iter().enumerate().skip(current) {
            let transaction = connection.transaction()?;
            transaction.execute_batch(script)?;
            transaction.pragma_update(None, USER_VERSION, index + 1)?;
            transaction.commit()?;
        }
        Ok(())
    }

    /// Schema version stored in the database.
    pub fn schema_version(&self) -> Result<usize> {
        Ok(self
            .lock()
            .pragma_query_value(None, USER_VERSION, |row| row.get(0))?)
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs a read with the connection.
    pub fn read<T>(&self, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        work(&self.lock())
    }

    /// Runs a write inside one transaction, committed when `work` succeeds.
    pub fn write<T>(&self, work: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        let value = work(&transaction)?;
        transaction.commit()?;
        Ok(value)
    }

    /// Deletes the index of a key that holds no memory, then compacts the database.
    pub fn purge_index(&self, project: &str) -> Result<PurgeReport> {
        let (chunks, files) = self.write(|transaction| {
            let memories: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1",
                params![project],
                |row| row.get(0),
            )?;
            if memories > 0 {
                return Err(MemoryError::Conflict(format!(
                    "`{project}` holds {memories} memories; its index is not purged"
                )));
            }
            let chunks =
                transaction.execute("DELETE FROM chunks WHERE project = ?1", params![project])?;
            let files =
                transaction.execute("DELETE FROM files WHERE project = ?1", params![project])?;
            Ok((chunks, files))
        })?;
        if chunks + files == 0 {
            return Err(MemoryError::NotFound(format!(
                "no index for the key `{project}`"
            )));
        }
        let compacted = self.lock().execute_batch(VACUUM).is_ok();
        Ok(PurgeReport {
            chunks,
            files,
            compacted,
        })
    }

    /// Counts and file size for one project.
    pub fn stats(&self, project: &str) -> Result<StoreStats> {
        let (chunks, files, memories) = self.read(|connection| {
            let count = |table: &str| -> Result<usize> {
                Ok(connection.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE project = ?1"),
                    params![project],
                    |row| row.get(0),
                )?)
            };
            Ok((count("chunks")?, count("files")?, count("memories")?))
        })?;
        let db_bytes = self
            .path
            .as_ref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(0, |metadata| metadata.len());
        Ok(StoreStats {
            chunks,
            files,
            memories,
            db_bytes,
        })
    }
}

impl From<std::num::ParseIntError> for MemoryError {
    fn from(error: std::num::ParseIntError) -> Self {
        MemoryError::Invalid(error.to_string())
    }
}

#[cfg(test)]
#[path = "../tests/unit/store_tests.rs"]
mod tests;
