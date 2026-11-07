use super::{providers_check_from, DoctorCheckName};
use aidp_agents::{AgentError, ProviderEntry, ProvidersConfig};

const CUSTOM_PROVIDER: &str = "custom";
const CUSTOM_PROGRAM: &str = "custom-bin";

fn config_with(id: &str, program: &str) -> ProvidersConfig {
    let mut config = ProvidersConfig::default();
    config.entries.insert(
        id.to_string(),
        ProviderEntry {
            program: Some(program.to_string()),
            ..ProviderEntry::default()
        },
    );
    config
}

#[test]
fn missing_binary_fails_and_names_provider_and_program() {
    let config = config_with(CUSTOM_PROVIDER, CUSTOM_PROGRAM);
    let lookup = |program: &str| (program == "claude").then(|| "1.0".to_string());
    let check = providers_check_from(Ok(config), &lookup);
    assert!(!check.ok);
    assert_eq!(check.name, DoctorCheckName::Providers);
    assert!(check.detail.contains(CUSTOM_PROVIDER));
    assert!(check.detail.contains(CUSTOM_PROGRAM));
    assert!(check.fix.is_some());
}

#[test]
fn all_binaries_present_is_ok() {
    let config = config_with(CUSTOM_PROVIDER, CUSTOM_PROGRAM);
    let lookup = |_: &str| Some("2.0".to_string());
    let check = providers_check_from(Ok(config), &lookup);
    assert!(check.ok);
    assert!(check
        .detail
        .contains(&format!("{CUSTOM_PROVIDER} ({CUSTOM_PROGRAM}) 2.0")));
}

#[test]
fn fake_provider_is_not_probed() {
    let config = config_with("fake", "nothing");
    let lookup = |program: &str| (program == "claude").then(|| "1.0".to_string());
    assert!(providers_check_from(Ok(config), &lookup).ok);
}

#[test]
fn invalid_config_fails_with_the_error_message() {
    let error = AgentError::Config("boom".into());
    let message = error.to_string();
    let check = providers_check_from(Err(error), &|_| None);
    assert!(!check.ok);
    assert_eq!(check.detail, message);
}
