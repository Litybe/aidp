use serde::{Deserialize, Serialize};

/// How many chunks hold a vector of the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChunks {
    pub total: usize,
    pub embedded: usize,
    /// Chunks holding a vector of another model.
    pub stale: usize,
}
