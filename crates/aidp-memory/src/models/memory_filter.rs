use crate::models::enums::{MemoryKind, MemoryStatus};

/// Criteria of `memory list`.
#[derive(Debug, Clone, Default)]
pub struct MemoryFilter {
    pub status: Option<MemoryStatus>,
    pub kind: Option<MemoryKind>,
    /// Free-text query matched on title and text.
    pub query: Option<String>,
}
