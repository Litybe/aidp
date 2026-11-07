use std::collections::BTreeMap;

#[derive(serde::Serialize)]
pub struct ClaudeMcpConfiguration {
    #[serde(rename = "mcpServers")]
    pub servers: BTreeMap<String, super::mcp_provider_server::McpProviderServer>,
}
