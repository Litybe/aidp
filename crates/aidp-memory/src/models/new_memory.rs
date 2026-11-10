use crate::models::constants::limits::DEFAULT_CONFIDENCE;
use crate::models::enums::{MemoryKind, MemoryStatus};
use crate::models::memory_ref::MemoryRef;
use crate::models::memory_scope::MemoryScope;

/// A memory about to be created, by a human or by a pipeline.
#[derive(Debug, Clone)]
pub struct NewMemory {
    pub project: String,
    pub kind: MemoryKind,
    pub title: String,
    pub text: String,
    pub scope: MemoryScope,
    pub refs: Vec<MemoryRef>,
    pub confidence: f64,
    /// `Proposed` or `Active`.
    pub status: MemoryStatus,
    /// Id of the memory this one replaces.
    pub supersedes: Option<String>,
    /// Written by an automatic source: active at once when confident enough.
    pub auto_activate: bool,
}

impl NewMemory {
    /// A proposal with the default confidence and an empty scope.
    pub fn proposal(
        project: impl Into<String>,
        kind: MemoryKind,
        title: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            project: project.into(),
            kind,
            title: title.into(),
            text: text.into(),
            scope: MemoryScope::default(),
            refs: Vec::new(),
            confidence: DEFAULT_CONFIDENCE,
            status: MemoryStatus::Proposed,
            supersedes: None,
            auto_activate: false,
        }
    }
}
