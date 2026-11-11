use serde::{Deserialize, Serialize};

/// Body of `PATCH /api/memory/model`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelPatchBody {
    pub autostart: bool,
}
