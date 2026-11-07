use serde::Serialize;

/// Host integration contracts understood and enforced before launching an agent.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayCapabilities {
    pub restricted_launch_policy: bool,
    pub headless_launch_policy: bool,
    pub execution_identity: bool,
    pub catalog: bool,
    pub mcp_configuration_references: bool,
    pub safe_human_rejection: bool,
    pub external_actions: bool,
    pub external_agents: bool,
}

impl GatewayCapabilities {
    /// Contracts enforced by the running gateway, independent of a flow's frozen definition.
    pub const CURRENT: Self = Self {
        restricted_launch_policy: true,
        headless_launch_policy: true,
        execution_identity: true,
        catalog: true,
        mcp_configuration_references: true,
        safe_human_rejection: true,
        external_actions: true,
        external_agents: true,
    };
}

#[cfg(test)]
#[path = "../../tests/unit/gateway_capabilities_tests.rs"]
mod tests;
