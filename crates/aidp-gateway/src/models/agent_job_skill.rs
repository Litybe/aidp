use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A skill the session must load, resolved to its `SKILL.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentJobSkill {
    pub name: String,
    pub path: PathBuf,
    pub required: bool,
}
