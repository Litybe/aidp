use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Default)]
pub struct RuntimeEnvironment {
    pub skill_root: Option<PathBuf>,
    pub initial_mcp_configurations: BTreeMap<String, aidp_agents::McpConfiguration>,
    pub child_environment: BTreeMap<String, String>,
    /// Web UI build served on `/`; `None` leaves every non-API path to the API routes.
    pub ui_dir: Option<PathBuf>,
}
