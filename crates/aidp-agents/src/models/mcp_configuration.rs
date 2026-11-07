use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{AgentError, McpServerConfig, McpTransport};

use super::constants::mcp::{
    CONFIGURATION_DEBUG, HTTPS_SCHEME, HTTP_SCHEME, INVALID_CONFIGURATION,
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfiguration {
    pub servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

impl std::fmt::Debug for McpConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(CONFIGURATION_DEBUG)
    }
}

impl McpConfiguration {
    pub fn validate(&self) -> Result<(), AgentError> {
        let mut names = BTreeSet::new();
        for server in &self.servers {
            if server.name.is_empty()
                || !server
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                || !names.insert(&server.name)
            {
                return Err(Self::invalid());
            }
            match server.transport {
                McpTransport::Http => {
                    let url = self.expand(server.url.as_deref().ok_or_else(Self::invalid)?)?;
                    if !(url.starts_with(HTTPS_SCHEME) || url.starts_with(HTTP_SCHEME))
                        || server.command.is_some()
                        || !server.args.is_empty()
                        || !server.env.is_empty()
                    {
                        return Err(Self::invalid());
                    }
                }
                McpTransport::Stdio => {
                    if self
                        .expand(server.command.as_deref().ok_or_else(Self::invalid)?)?
                        .trim()
                        .is_empty()
                        || server.url.is_some()
                        || !server.headers.is_empty()
                    {
                        return Err(Self::invalid());
                    }
                }
            }
            for value in server
                .headers
                .values()
                .chain(server.env.values())
                .chain(server.args.iter())
            {
                self.expand(value)?;
            }
        }
        Ok(())
    }

    pub fn expand(&self, value: &str) -> Result<String, AgentError> {
        let mut expanded = String::new();
        let mut remaining = value;
        while let Some(start) = remaining.find("${") {
            expanded.push_str(&remaining[..start]);
            let placeholder = &remaining[start + 2..];
            let end = placeholder.find('}').ok_or_else(Self::invalid)?;
            let name = &placeholder[..end];
            expanded.push_str(self.environment.get(name).ok_or_else(Self::invalid)?);
            remaining = &placeholder[end + 1..];
        }
        expanded.push_str(remaining);
        Ok(expanded)
    }

    fn invalid() -> AgentError {
        AgentError::Config(INVALID_CONFIGURATION.to_string())
    }
}
