//! Finds the embedding model on disk: `<gateway>/memory/models/<name>/`. Nothing is downloaded.

use crate::embedder::{Embedder, Model2VecEmbedder};
use crate::embedding_index::vector_counts;
use crate::error::Result;
use crate::models::constants::embedding::{DEFAULT_MODEL_NAME, MODELS_DIR, MODEL_FILES};
use crate::models::constants::paths::MEMORY_DIR;
use crate::models::model_status::ModelStatus;
use crate::store::MemoryStore;
use std::path::{Path, PathBuf};

/// Folder of the default model under a gateway directory.
pub fn model_dir(gateway_dir: &Path) -> PathBuf {
    gateway_dir
        .join(MEMORY_DIR)
        .join(MODELS_DIR)
        .join(DEFAULT_MODEL_NAME)
}

/// The model folder when it holds every model file, else `None`.
pub fn resolve_model(gateway_dir: &Path) -> Option<PathBuf> {
    let dir = model_dir(gateway_dir);
    MODEL_FILES
        .iter()
        .all(|file| dir.join(file).is_file())
        .then_some(dir)
}

/// Loads the default model; without it, or when it fails to load, warns once and returns `None` so callers fall back to BM25.
pub fn load_default_embedder(gateway_dir: &Path) -> Option<Box<dyn Embedder>> {
    let Some(dir) = resolve_model(gateway_dir) else {
        tracing::warn!(
            path = %model_dir(gateway_dir).display(),
            "embedding model not found; search is lexical only"
        );
        return None;
    };
    match Model2VecEmbedder::load(&dir) {
        Ok(embedder) => Some(Box::new(embedder)),
        Err(error) => {
            tracing::warn!(%error, "embedding model not loaded; search is lexical only");
            None
        }
    }
}

/// Size in bytes of the weights file of the default model: an estimate of the RAM it takes once loaded.
pub fn model_weights_bytes(gateway_dir: &Path) -> Option<u64> {
    let weights = model_dir(gateway_dir).join(MODEL_FILES[0]);
    std::fs::metadata(weights)
        .ok()
        .map(|metadata| metadata.len())
}

/// Presence, size and coverage of the default model; the size comes from the stored vectors, never from loading the model here.
pub fn model_status(store: &MemoryStore, gateway_dir: &Path) -> Result<ModelStatus> {
    let path = model_dir(gateway_dir);
    let present = resolve_model(gateway_dir).is_some();
    let (embedded_chunks, stale_chunks, total_chunks, stored_dim) = store.read(|connection| {
        let (embedded, stale, total) = vector_counts(connection, DEFAULT_MODEL_NAME)?;
        let dim: Option<i64> = connection
            .query_row(
                "SELECT dim FROM chunk_vectors WHERE model = ?1 LIMIT 1",
                [DEFAULT_MODEL_NAME],
                |row| row.get(0),
            )
            .ok();
        Ok((embedded, stale, total, dim))
    })?;
    let dim = stored_dim.map(|dim| dim as usize);
    Ok(ModelStatus {
        model: DEFAULT_MODEL_NAME.to_string(),
        path,
        present,
        dim,
        embedded_chunks,
        stale_chunks,
        total_chunks,
    })
}

#[cfg(test)]
#[path = "../tests/unit/model_resolver_tests.rs"]
mod tests;
