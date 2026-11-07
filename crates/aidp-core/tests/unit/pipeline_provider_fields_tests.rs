use super::*;

const FIXTURE: &str = include_str!("../fixtures/pipeline_with_providers.yaml");
const PLAIN: &str = include_str!("../fixtures/pipeline_provider_fields.yaml");
const EXECUTOR_ROLE: &str = "executor";
const PROVIDER_KEY: &str = "provider";

/// Provider fields of a stage and of the settings map deserialize from YAML.
#[test]
fn provider_fields_are_parsed() {
    let pipeline: Pipeline = serde_yaml::from_str(FIXTURE).unwrap();
    assert_eq!(pipeline.stages[0].provider.as_deref(), Some("fake"));
    assert_eq!(
        pipeline
            .settings
            .providers
            .get(EXECUTOR_ROLE)
            .map(String::as_str),
        Some("claude")
    );
}

/// A pipeline declaring no provider serializes without any provider key.
#[test]
fn absent_provider_fields_are_not_serialized() {
    let pipeline: Pipeline = serde_yaml::from_str(PLAIN).unwrap();
    let yaml = serde_yaml::to_string(&pipeline).unwrap();
    assert!(!yaml.contains(PROVIDER_KEY));
}
