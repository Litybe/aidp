use std::path::Path;

use crate::models::enums::agent_error::AgentError;
use crate::models::enums::provider_origin::ProviderOrigin;
pub use crate::models::providers_config::ProvidersConfig;

pub const CLAUDE_PROVIDER: &str = "claude";
pub const FAKE_PROVIDER: &str = "fake";
pub const BUILTIN_PROVIDERS: &[&str] = &[CLAUDE_PROVIDER, FAKE_PROVIDER];
pub const PROVIDERS_KEY: &str = "providers";
pub const PROVIDER_VERSION_ARG: &str = "--version";
const AIDP_DIR: &str = ".aidp";
const CONFIG_FILE: &str = "config.json";

impl ProvidersConfig {
    /// Reads and validates the `providers` key; absent means the default config.
    pub fn from_config_json(config: &serde_json::Value) -> Result<Self, AgentError> {
        let parsed = match config.get(PROVIDERS_KEY) {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|error| AgentError::Config(error.to_string()))?,
            None => Self::default(),
        };
        parsed.validate()?;
        Ok(parsed)
    }

    /// Loads `<home>/.aidp/config.json`; a missing file means the default config.
    pub fn load(home: &Path) -> Result<Self, AgentError> {
        let path = home.join(AIDP_DIR).join(CONFIG_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)?;
        let config: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| AgentError::Config(error.to_string()))?;
        Self::from_config_json(&config)
    }

    /// True when the id is built in or declared in this config.
    pub fn knows(&self, id: &str) -> bool {
        BUILTIN_PROVIDERS.contains(&id) || self.entries.contains_key(id)
    }

    /// The configured default id, or `claude`.
    pub fn default_id(&self) -> &str {
        self.default.as_deref().unwrap_or(CLAUDE_PROVIDER)
    }

    /// Refuses a `default` naming an unknown provider.
    pub fn validate(&self) -> Result<(), AgentError> {
        match &self.default {
            Some(name) if !self.knows(name) => Err(AgentError::UnknownProvider {
                name: name.clone(),
                origin: ProviderOrigin::ConfigDefault,
            }),
            _ => Ok(()),
        }
    }

    /// Program a provider runs: its configured `program`, else its id; `None` for `fake`.
    pub fn program_for(&self, id: &str) -> Option<String> {
        if id == FAKE_PROVIDER {
            return None;
        }
        let configured = self.entries.get(id).and_then(|entry| entry.program.clone());
        Some(configured.unwrap_or_else(|| id.to_string()))
    }

    /// Providers to probe: `claude` first, then every other configured one but `fake`.
    pub fn providers_to_probe(&self) -> Vec<(String, String)> {
        let configured = self
            .entries
            .keys()
            .map(String::as_str)
            .filter(|id| *id != CLAUDE_PROVIDER);
        std::iter::once(CLAUDE_PROVIDER)
            .chain(configured)
            .filter_map(|id| {
                self.program_for(id)
                    .map(|program| (id.to_string(), program))
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "../tests/unit/config_tests.rs"]
mod config_tests;
