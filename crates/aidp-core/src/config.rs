//! Project & user `aidp.toml` config (port from v3 `aidp-config.md`).
//! Stub — to port from v3 `crates/aidp-core/src/config.rs`.

use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct AidpConfig {
    pub mcp_fallback: bool,
    pub agents_dir: Vec<String>,
    pub skills_dir: Vec<String>,
    pub project_context_file: Option<String>,
    pub decision_records_dir: Option<String>,
    pub jira: Option<JiraCfg>,
    pub gh: Option<GhCfg>,
    pub confluence: Option<ConfluenceCfg>,
    pub teams: Option<TeamsCfg>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct JiraCfg {
    pub base_url: String,
    pub project_keys: Vec<String>,
}
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct GhCfg {
    pub host: String,
    pub default_repo: Option<String>,
}
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct ConfluenceCfg {
    pub base_url: String,
    pub default_space: String,
}
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct TeamsCfg {
    pub webhook_env_var: String,
}

pub fn load(_explicit_path: Option<&std::path::Path>) -> anyhow::Result<AidpConfig> {
    // TODO: port from v3 (project + user merge, env override).
    Ok(AidpConfig::default())
}
