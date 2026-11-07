use crate::models::enums::Source;
use crate::models::index_root::IndexRoot;

/// What `Indexer::index` should do.
#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    /// Sources to refresh; empty means every file source.
    pub sources: Vec<Source>,
    /// Re-chunk every file even when its mtime and hash are unchanged.
    pub full: bool,
    /// Directories feeding `run`, `ticket` and `knowledge`.
    pub extra_roots: Vec<IndexRoot>,
}
