//! Loads the flow configuration: `.aidp/flow.json` of the workspace over the `flow` key of the user config.

use crate::models::constants::flow_repo::{
    AIDP_DIR, FLOW_CONFIG_FILE, NOT_AN_OBJECT, PARSE_ERROR_PREFIX, READ_ERROR_PREFIX, USER_FLOW_KEY,
};
use crate::models::flow_repo_config::FlowRepoConfig;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// `<workspace>/.aidp/flow.json`.
pub fn config_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(AIDP_DIR).join(FLOW_CONFIG_FILE)
}

/// Overlays `over` on `base`: objects merge key by key, any other value replaces.
pub fn merge(base: Value, over: Value) -> Value {
    match (base, over) {
        (Value::Object(mut base_map), Value::Object(over_map)) => {
            for (key, over_value) in over_map {
                let merged = match base_map.remove(&key) {
                    Some(base_value) => merge(base_value, over_value),
                    None => over_value,
                };
                base_map.insert(key, merged);
            }
            Value::Object(base_map)
        }
        (_, over_value) => over_value,
    }
}

fn parse_object(text: &str, origin: &str) -> anyhow::Result<Value> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| anyhow::anyhow!("{PARSE_ERROR_PREFIX} {origin}: {error}"))?;
    if value.is_object() {
        Ok(value)
    } else {
        anyhow::bail!("{origin} {NOT_AN_OBJECT}")
    }
}

/// Builds the configuration from the text of the user config and of the repo file, either of which may be absent.
pub fn parse(user_text: Option<&str>, repo_text: Option<&str>) -> anyhow::Result<FlowRepoConfig> {
    let mut merged = Value::Object(Default::default());
    if let Some(text) = user_text {
        let user = parse_object(text, USER_FLOW_KEY)?;
        if let Some(defaults) = user.get(USER_FLOW_KEY) {
            merged = merge(merged, defaults.clone());
        }
    }
    if let Some(text) = repo_text {
        merged = merge(merged, parse_object(text, FLOW_CONFIG_FILE)?);
    }
    serde_json::from_value(merged)
        .map_err(|error| anyhow::anyhow!("{PARSE_ERROR_PREFIX} {FLOW_CONFIG_FILE}: {error}"))
}

fn read_optional(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => anyhow::bail!("{READ_ERROR_PREFIX} {}: {error}", path.display()),
    }
}

/// Loads the configuration of `workspace_root`; missing files give the defaults.
pub fn load(workspace_root: &Path) -> anyhow::Result<FlowRepoConfig> {
    let user_text = match aidp_core::vault::config_file() {
        Some(path) => read_optional(&path)?,
        None => None,
    };
    let repo_text = read_optional(&config_path(workspace_root))?;
    parse(user_text.as_deref(), repo_text.as_deref())
}

#[cfg(test)]
#[path = "tests/units/flow_repo_tests.rs"]
mod flow_repo_tests;
