use crate::models::constants::mcp_configuration::{
    INVALID_CONFIGURATION, INVALID_ID, MAX_ID_BYTES, UNKNOWN_REFERENCE,
};
use crate::models::mcp_configuration_registry::McpConfigurationRegistry;
use crate::sync::lock;
pub use aidp_agents::McpConfiguration;
use std::sync::Arc;

impl McpConfigurationRegistry {
    /// Replaces one named configuration atomically without changing existing launch snapshots.
    pub fn upsert(
        &self,
        configuration_id: String,
        configuration: McpConfiguration,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !configuration_id.is_empty()
                && configuration_id.len() <= MAX_ID_BYTES
                && configuration_id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_' | '.')),
            INVALID_ID
        );
        configuration
            .validate()
            .map_err(|_| anyhow::anyhow!(INVALID_CONFIGURATION))?;
        lock(&self.configurations).insert(configuration_id, Arc::new(configuration));
        Ok(())
    }

    /// Resolves a supplied reference or refuses it without exposing configuration values.
    pub fn resolve(
        &self,
        configuration_id: Option<&str>,
    ) -> anyhow::Result<Option<Arc<McpConfiguration>>> {
        configuration_id
            .map(|configuration_id| {
                lock(&self.configurations)
                    .get(configuration_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!(UNKNOWN_REFERENCE))
            })
            .transpose()
    }
}

#[cfg(test)]
#[path = "../tests/unit/mcp_configurations_tests.rs"]
mod tests;
