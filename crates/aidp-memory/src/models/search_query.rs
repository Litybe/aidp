use crate::models::constants::limits::DEFAULT_K;
use crate::models::enums::Source;

/// A search request against the index.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub project: String,
    pub text: String,
    /// Sources to search; empty means all.
    pub sources: Vec<Source>,
    /// Glob applied to the chunk path.
    pub path_glob: Option<String>,
    pub k: usize,
}

impl SearchQuery {
    /// A query over every source with the default result count.
    pub fn new(project: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            text: text.into(),
            sources: Vec::new(),
            path_glob: None,
            k: DEFAULT_K,
        }
    }
}
