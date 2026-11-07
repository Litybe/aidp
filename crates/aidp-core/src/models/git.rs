//! Data carried by the recording [`crate::git::Git`] fake.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use crate::gitlog::Commit;

/// Per-method call counters for [`RecordingGit`].
#[derive(Debug, Default)]
pub struct GitCallCounts {
    pub head: AtomicUsize,
    pub branch: AtomicUsize,
    pub log: AtomicUsize,
    pub changed: AtomicUsize,
    pub last_commit_date: AtomicUsize,
    pub is_usable: AtomicUsize,
    pub worktree_list: AtomicUsize,
}

/// A fake [`crate::git::Git`] returning canned values and counting calls.
/// Public so other crates' tests can use it too.
#[derive(Default)]
pub struct RecordingGit {
    pub calls: GitCallCounts,
    pub head_value: Option<String>,
    pub branch_value: Option<String>,
    pub log_value: Vec<Commit>,
    pub changed_value: Vec<String>,
    pub last_commit_dates: Mutex<BTreeMap<String, String>>,
    pub usable: bool,
    pub worktrees: Vec<PathBuf>,
}

impl RecordingGit {
    /// Every canned value empty and `usable: false`.
    pub fn new() -> Self {
        Self::default()
    }
}
