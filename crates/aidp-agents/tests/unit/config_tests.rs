use crate::config::{CLAUDE_PROVIDER, FAKE_PROVIDER};
use crate::models::enums::agent_error::AgentError;
use crate::models::providers_config::ProvidersConfig;

const FIXTURE: &str = include_str!("../fixtures/providers_config.json");
const EMPTY_FIXTURE: &str = include_str!("../fixtures/providers_config_empty.json");
const UNKNOWN_DEFAULT_FIXTURE: &str =
    include_str!("../fixtures/providers_config_unknown_default.json");
const WITH_FAKE_FIXTURE: &str = include_str!("../fixtures/providers_config_with_fake.json");
const OTHER_PROVIDER: &str = "other";
const OTHER_PROGRAM: &str = "other-cli";
const ZETA_PROVIDER: &str = "zeta";
const UNKNOWN_PROVIDER: &str = "ghost";

/// Parses a fixture into a validated config.
fn config_from(fixture: &str) -> ProvidersConfig {
    let json: serde_json::Value = serde_json::from_str(fixture).unwrap();
    ProvidersConfig::from_config_json(&json).unwrap()
}

/// The providers key of the fixture is read with its models and args.
#[test]
fn parses_fixture_providers_key() {
    let config = config_from(FIXTURE);
    assert_eq!(config.default_id(), CLAUDE_PROVIDER);
    assert_eq!(config.entries["claude"].models["opus"], "claude-opus-5-5");
    assert_eq!(config.entries[OTHER_PROVIDER].args, vec!["--json"]);
}

/// An absent providers key yields the default config.
#[test]
fn absent_key_gives_default_config() {
    let config = config_from(EMPTY_FIXTURE);
    assert_eq!(config.default_id(), CLAUDE_PROVIDER);
    assert!(config.entries.is_empty());
}

/// A default naming an unknown provider is refused.
#[test]
fn unknown_default_is_refused() {
    let json: serde_json::Value = serde_json::from_str(UNKNOWN_DEFAULT_FIXTURE).unwrap();
    let error = ProvidersConfig::from_config_json(&json).unwrap_err();
    assert!(matches!(error, AgentError::UnknownProvider { .. }));
    assert!(error.to_string().contains(UNKNOWN_PROVIDER));
}

/// A missing config file yields the default config.
#[test]
fn missing_file_gives_default_config() {
    let dir = std::env::temp_dir().join("aidp-agents-no-config");
    let config = ProvidersConfig::load(&dir).unwrap();
    assert_eq!(config, ProvidersConfig::default());
}

/// The program is the configured one, else the id, and none for fake.
#[test]
fn program_for_uses_entry_then_id_and_skips_fake() {
    let config = config_from(FIXTURE);
    assert_eq!(
        config.program_for(OTHER_PROVIDER).as_deref(),
        Some(OTHER_PROGRAM)
    );
    assert_eq!(
        config.program_for(UNKNOWN_PROVIDER).as_deref(),
        Some(UNKNOWN_PROVIDER)
    );
    assert_eq!(config.program_for(FAKE_PROVIDER), None);
}

/// Claude comes first, then configured entries, never fake.
#[test]
fn providers_to_probe_lists_claude_first_without_fake() {
    let expected = vec![
        (CLAUDE_PROVIDER.to_string(), CLAUDE_PROVIDER.to_string()),
        (OTHER_PROVIDER.to_string(), OTHER_PROGRAM.to_string()),
        (ZETA_PROVIDER.to_string(), ZETA_PROVIDER.to_string()),
    ];
    assert_eq!(
        config_from(WITH_FAKE_FIXTURE).providers_to_probe(),
        expected
    );
}

/// An empty config probes claude alone.
#[test]
fn providers_to_probe_defaults_to_claude() {
    let expected = vec![(CLAUDE_PROVIDER.to_string(), CLAUDE_PROVIDER.to_string())];
    assert_eq!(config_from(EMPTY_FIXTURE).providers_to_probe(), expected);
}
