use std::collections::BTreeMap;

/// Dirty paths of a worktree with a hash of their bytes, captured before a plan task runs.
/// Keys are repo-relative paths as git prints them; `None` marks a deleted file.
/// `contents` keeps the pre-task bytes of each dirty path so a stray write can be undone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskWriteBaseline {
    pub head: Option<String>,
    pub hashes: BTreeMap<String, Option<u64>>,
    pub contents: BTreeMap<String, Option<Vec<u8>>>,
    /// Folder inside the worktree the guard leaves alone, as git prints it: the run directory.
    pub exempt_prefix: Option<String>,
}
