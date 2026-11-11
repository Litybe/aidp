//! Text embeddings: the `Embedder` trait and its Model2Vec static-embedding implementation.

use crate::error::{MemoryError, Result};
use crate::models::shared_embedder::SharedEmbedder;
use model2vec_rs::model::StaticModel;
use std::path::Path;

/// Turns texts into fixed-size vectors; the same model must embed documents and queries.
pub trait Embedder: Send + Sync {
    /// Identifier stored next to every vector; a vector of another model is re-embedded.
    fn model_id(&self) -> &str;
    /// Number of components of every vector.
    fn dim(&self) -> usize;
    /// One vector per text, in order.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
}

/// A Model2Vec static-embedding model loaded from a local folder, never downloaded.
pub struct Model2VecEmbedder {
    model: StaticModel,
    model_id: String,
}

impl Model2VecEmbedder {
    /// Loads `model.safetensors`, `tokenizer.json` and `config.json` from `dir`; the folder name is the model id.
    pub fn load(dir: &Path) -> Result<Self> {
        let model = StaticModel::from_pretrained(dir, None, None, None)
            .map_err(|error| MemoryError::Embedding(format!("{}: {error}", dir.display())))?;
        let model_id = dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(Self { model, model_id })
    }
}

impl Embedder for Model2VecEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.model.dim()
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let owned: Vec<String> = texts.iter().map(|text| (*text).to_string()).collect();
        Ok(self.model.encode(&owned))
    }
}

impl Embedder for SharedEmbedder {
    fn model_id(&self) -> &str {
        self.0.model_id()
    }

    fn dim(&self) -> usize {
        self.0.dim()
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.0.embed(texts)
    }
}
