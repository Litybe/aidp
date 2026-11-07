//! Vault value types.

use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultSource {
    /// `--vault <path>`.
    Flag,
    /// `AIDP_VAULT`.
    Env,
    /// `~/.aidp/config.json`.
    Config,
    /// Found by searching outwards from the workspace.
    Autodetect,
}

#[derive(Debug, Clone, Serialize)]
pub struct Vault {
    pub root: PathBuf,
    pub source: VaultSource,
    /// Set when auto-detection chose between several candidate vaults.
    /// Surfaced by `aidp vault status` so an ambiguous pick is visible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambiguity: Option<String>,
}
