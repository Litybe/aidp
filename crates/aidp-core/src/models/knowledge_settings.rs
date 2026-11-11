use crate::models::constants::knowledge_context::{
    DEFAULT_KNOWLEDGE_BUDGET_TOKENS, SETTINGS_KNOWLEDGE_KEY,
};
use crate::models::enums::KnowledgeRoute;
use crate::models::pipeline::Settings;
use serde::Deserialize;

/// `settings.knowledge`: the compile-time knowledge block of the agent stages.
///
/// Written `knowledge: true` or as a mapping; the mapping may also hold the prompt-rendering keys
/// of the project knowledge base, which this type ignores.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KnowledgeSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Token budget of the block of each stage.
    #[serde(default = "default_budget")]
    pub budget: usize,
    #[serde(default)]
    pub route: Option<KnowledgeRoute>,
    /// Stages that get the block; every agent stage when absent.
    #[serde(default)]
    pub stages: Option<Vec<String>>,
}

fn default_budget() -> usize {
    DEFAULT_KNOWLEDGE_BUDGET_TOKENS
}

impl Default for KnowledgeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            budget: default_budget(),
            route: None,
            stages: None,
        }
    }
}

impl KnowledgeSettings {
    /// The knowledge settings of a pipeline; disabled when absent or malformed.
    pub fn from_settings(settings: &Settings) -> Self {
        match settings.extra.get(SETTINGS_KNOWLEDGE_KEY) {
            Some(serde_yaml::Value::Bool(enabled)) => Self {
                enabled: *enabled,
                ..Self::default()
            },
            Some(value @ serde_yaml::Value::Mapping(_)) => {
                serde_yaml::from_value(value.clone()).unwrap_or_default()
            }
            _ => Self::default(),
        }
    }

    /// True when `stage` is meant to get the block.
    pub fn covers(&self, stage: &str) -> bool {
        self.enabled
            && self
                .stages
                .as_ref()
                .is_none_or(|stages| stages.iter().any(|name| name == stage))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/knowledge_settings_tests.rs"]
mod tests;
