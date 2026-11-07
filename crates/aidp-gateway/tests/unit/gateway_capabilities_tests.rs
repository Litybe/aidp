use super::*;
use crate::models::constants::runtime_api::SAFE_HUMAN_REJECTION;

const RETIRED_ENVIRONMENT_APPROVAL: &str = "manualEnvironmentApproval";

/// The running gateway advertises safe human rejection with its stable wire name.
#[test]
fn current_gateway_advertises_safe_human_rejection() {
    let value = serde_json::to_value(GatewayCapabilities::CURRENT).unwrap();
    assert_eq!(value[SAFE_HUMAN_REJECTION], true);
    assert!(value.get(RETIRED_ENVIRONMENT_APPROVAL).is_none());
}

/// The running gateway advertises the external action executor with its stable wire name.
#[test]
fn current_gateway_advertises_external_actions() {
    let value = serde_json::to_value(GatewayCapabilities::CURRENT).unwrap();
    assert_eq!(value["externalActions"], true);
}

/// The running gateway advertises host-run agent sessions with their stable wire name.
#[test]
fn current_gateway_advertises_external_agents() {
    let value = serde_json::to_value(GatewayCapabilities::CURRENT).unwrap();
    assert_eq!(value["externalAgents"], true);
}
