use std::collections::BTreeSet;

/// Workspace state captured before a stage: the HEAD commit and the paths already dirty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeBaseline {
    pub head: Option<String>,
    pub dirty_paths: BTreeSet<String>,
    /// Folder inside the workspace the guard leaves alone, as git prints it: the run directory.
    pub exempt_prefix: Option<String>,
}
