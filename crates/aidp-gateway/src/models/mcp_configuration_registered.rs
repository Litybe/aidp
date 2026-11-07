use serde::Serialize;

#[derive(Serialize)]
pub struct McpConfigurationRegistered {
    pub id: String,
}
