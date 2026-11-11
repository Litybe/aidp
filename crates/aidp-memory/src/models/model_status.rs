use serde::Serialize;
use std::path::PathBuf;

/// State of the embedding model and of the vectors it produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub model: String,
    pub path: PathBuf,
    /// The model folder holds every file of a Model2Vec model.
    pub present: bool,
    /// Vector size, read from the stored vectors or the model; `None` when unknown.
    pub dim: Option<usize>,
    /// Chunks holding a vector of this model.
    pub embedded_chunks: usize,
    /// Chunks holding a vector of another model, to be re-embedded.
    pub stale_chunks: usize,
    /// Chunks of the database.
    pub total_chunks: usize,
}
