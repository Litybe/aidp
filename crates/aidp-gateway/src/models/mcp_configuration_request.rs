use aidp_agents::McpConfiguration;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct McpConfigurationRequest {
    pub id: String,
    #[serde(flatten)]
    pub configuration: McpConfiguration,
}
