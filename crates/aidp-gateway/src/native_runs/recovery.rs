//! Run index and restart recovery: a done stage is never replayed, a stage that was running is
//! relaunched once with a resume instruction, a gate that was pending stays pending.

use super::state::RunStore;
use super::NativeRunManager;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::store::{read_json, write_json};
use crate::sync::lock;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::warn;

const INDEX_DIR: &str = "native-runs";
const INDEX_FILE: &str = "index.json";

/// Serialises index rewrites inside the daemon.
static INDEX_GUARD: Mutex<()> = Mutex::new(());

/// Location of the run index under the gateway state directory.
pub fn index_path(state_dir: &Path) -> PathBuf {
    state_dir.join(INDEX_DIR).join(INDEX_FILE)
}

/// Every run the daemon knows, as run id to run directory.
pub fn index_read(file: &Path) -> BTreeMap<String, PathBuf> {
    let _guard = lock(&INDEX_GUARD);
    read_json(file, BTreeMap::new())
}

/// Adds a run to the index.
pub fn index_add(file: &Path, run_id: &str, run_dir: &Path) -> anyhow::Result<()> {
    let _guard = lock(&INDEX_GUARD);
    let mut index: BTreeMap<String, PathBuf> = read_json(file, BTreeMap::new());
    index.insert(run_id.to_string(), run_dir.to_path_buf());
    write_json(file, &index)
}

/// True for a status a restart picks up again; a paused run waits for a human, a final one is over.
pub fn is_resumable(status: NativeRunStatus) -> bool {
    matches!(
        status,
        NativeRunStatus::Pending | NativeRunStatus::Running | NativeRunStatus::WaitingGate
    )
}

/// Reloads every run that was executing when the daemon stopped and starts it again.
/// Call once at daemon start. Returns how many runs were resumed.
pub fn resume_all(manager: &Arc<NativeRunManager>) -> usize {
    let mut resumed = 0;
    for (run_id, run_dir) in index_read(&manager.deps().index_file) {
        let store = match RunStore::open(&run_dir) {
            Ok(store) => store,
            Err(error) => {
                warn!(run = %run_id, %error, "native run not reloaded");
                continue;
            }
        };
        if !is_resumable(store.status()) {
            continue;
        }
        if let Err(error) = store.prepare_resume() {
            warn!(run = %run_id, %error, "native run not prepared for resume");
            continue;
        }
        match manager.adopt(Arc::new(store)) {
            Ok(()) => resumed += 1,
            Err(error) => warn!(run = %run_id, %error, "native run not resumed"),
        }
    }
    resumed
}
