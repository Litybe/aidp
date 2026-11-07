use crate::models::enums::{MemoryKind, MemoryStatus};
use crate::models::memory_ref::MemoryRef;
use crate::models::memory_scope::MemoryScope;
use serde::{Deserialize, Serialize};

/// A durable lesson of the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub project: String,
    pub kind: MemoryKind,
    pub title: String,
    pub text: String,
    pub scope: MemoryScope,
    pub refs: Vec<MemoryRef>,
    pub confidence: f64,
    pub evidence: u32,
    pub status: MemoryStatus,
    pub supersedes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
    pub uses: u32,
}
