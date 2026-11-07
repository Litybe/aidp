use super::unknown_provider_errors;
use aidp_agents::ProvidersConfig;

const UNKNOWN_STAGE: &str = include_str!("../fixtures/pipeline_unknown_stage_provider.yaml");
const UNKNOWN_SETTINGS: &str = include_str!("../fixtures/pipeline_unknown_settings_provider.yaml");
const KNOWN: &str = include_str!("../fixtures/pipeline_known_provider.yaml");
const UNKNOWN_ID: &str = "nonexistent-provider";
const STAGE_NAME: &str = "build";
const SETTINGS_ROLE: &str = "settings.providers.executor";

/// Parses a fixture into a pipeline.
fn parse(yaml: &str) -> aidp_core::pipeline::Pipeline {
    serde_yaml::from_str(yaml).unwrap()
}

/// An unknown stage provider is rejected, naming provider and stage.
#[test]
fn unknown_stage_provider_names_provider_and_stage() {
    let error = unknown_provider_errors(&parse(UNKNOWN_STAGE), &ProvidersConfig::default())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(UNKNOWN_ID) && error.contains(STAGE_NAME),
        "{error}"
    );
}

/// An unknown settings provider is rejected, naming provider and role.
#[test]
fn unknown_settings_provider_names_provider_and_role() {
    let error = unknown_provider_errors(&parse(UNKNOWN_SETTINGS), &ProvidersConfig::default())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(UNKNOWN_ID) && error.contains(SETTINGS_ROLE),
        "{error}"
    );
}

/// Built-in providers pass.
#[test]
fn known_providers_pass() {
    assert!(unknown_provider_errors(&parse(KNOWN), &ProvidersConfig::default()).is_ok());
}
