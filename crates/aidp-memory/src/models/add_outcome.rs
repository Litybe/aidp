use crate::models::memory::Memory;
use serde::{Deserialize, Serialize};

/// What `MemoryManager::add` did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddOutcome {
    pub memory: Memory,
    /// The proposal matched an existing memory, whose evidence grew.
    pub deduplicated: bool,
    /// The memory became active during this call.
    pub activated: bool,
}
