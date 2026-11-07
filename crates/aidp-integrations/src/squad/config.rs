//! Loading of the optional `squad` key of `~/.aidp/config.json`.

use crate::models::constants::squad::{CONFIG_FILE_DIR, CONFIG_FILE_NAME, SQUAD_CONFIG_KEY};
use crate::models::squad_config::SquadConfig;
use std::path::Path;

/// The squad config of the current user; defaults when absent or unreadable.
pub fn load_config() -> SquadConfig {
    aidp_core::vault::home_dir()
        .map(|home| load_config_from(&home.join(CONFIG_FILE_DIR).join(CONFIG_FILE_NAME)))
        .unwrap_or_default()
}

/// The squad config stored in `path`; defaults when the file or key is missing or malformed.
pub fn load_config_from(path: &Path) -> SquadConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|root| root.get(SQUAD_CONFIG_KEY).cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../tests/unit/config_tests.rs"]
mod tests;
