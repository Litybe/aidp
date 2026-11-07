use crate::models::constants::squad::{
    BOT_DEPENDABOT, BOT_RENOVATE, DEFAULT_GITHUB_TEAM, DEFAULT_SQUAD_NAME,
};
use serde::{Deserialize, Serialize};

/// Squad settings read from the `squad` key of the user config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SquadConfig {
    pub name: String,
    pub github_team: String,
    pub extra_repos: Vec<String>,
    pub exclude_repos: Vec<String>,
    pub bot_authors: Vec<String>,
}

impl Default for SquadConfig {
    fn default() -> Self {
        Self {
            name: DEFAULT_SQUAD_NAME.to_string(),
            github_team: DEFAULT_GITHUB_TEAM.to_string(),
            extra_repos: Vec::new(),
            exclude_repos: Vec::new(),
            bot_authors: vec![BOT_DEPENDABOT.to_string(), BOT_RENOVATE.to_string()],
        }
    }
}
