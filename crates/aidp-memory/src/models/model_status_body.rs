use crate::models::enums::model_state::ModelState;
use crate::models::model_chunks::ModelChunks;
use serde::{Deserialize, Serialize};

/// Body of `GET /api/memory/model` and payload of the `memory.model` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusBody {
    pub model: String,
    pub installed: bool,
    pub state: ModelState,
    pub autostart: bool,
    pub dim: Option<usize>,
    /// Estimate: the size of `model.safetensors`, set while the model is loading or running.
    pub ram_bytes: Option<u64>,
    /// RFC 3339 instant the model finished loading.
    pub loaded_at: Option<String>,
    pub error: Option<String>,
    pub chunks: ModelChunks,
    pub path: String,
}
