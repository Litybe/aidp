use serde::{Deserialize, Serialize};

/// Content of `memory/model.json`: what the operator chose for the model service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSettingsFile {
    pub autostart: bool,
}
