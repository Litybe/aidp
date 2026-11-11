use serde::{Deserialize, Serialize};

/// Answer of `POST /api/embed`: one vector per text, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedResponse {
    pub model: String,
    pub dim: usize,
    pub vectors: Vec<Vec<f32>>,
}
