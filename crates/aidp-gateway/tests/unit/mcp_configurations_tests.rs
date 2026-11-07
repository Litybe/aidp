use super::*;
use crate::models::constants::mcp_configuration::{INVALID_ID, UNKNOWN_REFERENCE};
use std::collections::BTreeMap;

const CONFIGURATION_ID: &str = "ada";
const ENVIRONMENT_KEY: &str = "MCP_PRIVATE_VALUE";
const FIRST_SECRET: &str = "first-private-value";
const NEXT_SECRET: &str = "next-private-value";
const INVALID_CONFIGURATION_ID: &str = "private/value";

fn configuration(secret: &str) -> McpConfiguration {
    McpConfiguration {
        servers: Vec::new(),
        environment: BTreeMap::from([(ENVIRONMENT_KEY.to_string(), secret.to_string())]),
    }
}

#[test]
fn replacement_preserves_existing_launch_snapshot() {
    let registry = McpConfigurationRegistry::default();
    registry
        .upsert(CONFIGURATION_ID.to_string(), configuration(FIRST_SECRET))
        .unwrap();
    let existing = registry.resolve(Some(CONFIGURATION_ID)).unwrap().unwrap();
    registry
        .upsert(CONFIGURATION_ID.to_string(), configuration(NEXT_SECRET))
        .unwrap();
    let next = registry.resolve(Some(CONFIGURATION_ID)).unwrap().unwrap();
    assert_eq!(existing.environment[ENVIRONMENT_KEY], FIRST_SECRET);
    assert_eq!(next.environment[ENVIRONMENT_KEY], NEXT_SECRET);
    assert!(!Arc::ptr_eq(&existing, &next));
}

#[test]
fn unavailable_reference_never_falls_back_to_ambient_configuration() {
    let registry = McpConfigurationRegistry::default();
    assert!(registry.resolve(None).unwrap().is_none());
    let error = registry.resolve(Some(CONFIGURATION_ID)).unwrap_err();
    assert_eq!(error.to_string(), UNKNOWN_REFERENCE);
}

#[test]
fn invalid_identifier_is_refused_without_echoing_values() {
    let registry = McpConfigurationRegistry::default();
    let error = registry
        .upsert(
            INVALID_CONFIGURATION_ID.to_string(),
            configuration(FIRST_SECRET),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), INVALID_ID);
    assert!(registry.resolve(Some(INVALID_CONFIGURATION_ID)).is_err());
}
