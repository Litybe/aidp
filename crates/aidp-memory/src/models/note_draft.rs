use crate::models::enums::{Domain, NoteStatus};

/// Everything needed to write a knowledge note file.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteDraft {
    pub domain: Domain,
    pub tags: Vec<String>,
    pub agents: Vec<String>,
    pub status: NoteStatus,
    pub importance: f64,
    pub updated: String,
    pub source: String,
    pub title: String,
    pub body: String,
    /// Extra frontmatter lines after `source`, as `(key, raw value)`.
    pub extra: Vec<(String, String)>,
    /// Bullets of the `## Evidence` section.
    pub evidence: Vec<String>,
}
