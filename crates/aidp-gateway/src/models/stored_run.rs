use crate::vault_index::RunSummary;
use std::path::PathBuf;
use std::sync::Arc;

/// One run of the vault index as the SQLite cache stores it, keyed by its directory.
#[derive(Clone, Debug)]
pub struct StoredRun {
    pub run_dir: PathBuf,
    pub ticket: String,
    pub run: Arc<RunSummary>,
}
