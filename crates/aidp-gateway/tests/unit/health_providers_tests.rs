use crate::health::providers_value;
use aidp_agents::{ProviderEntry, ProvidersConfig};

const CUSTOM_PROVIDER: &str = "custom";
const CUSTOM_PROGRAM: &str = "custom-bin";

#[test]
fn providers_rows_carry_id_program_found_and_version() {
    let mut config = ProvidersConfig::default();
    config.entries.insert(
        CUSTOM_PROVIDER.into(),
        ProviderEntry {
            program: Some(CUSTOM_PROGRAM.into()),
            ..ProviderEntry::default()
        },
    );
    config
        .entries
        .insert("fake".into(), ProviderEntry::default());
    let probe = |program: &str| (program == "claude").then(|| "1.2.3".to_string());

    let rows = providers_value(&config, probe);
    let rows = rows.as_array().expect("an array");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "claude");
    assert_eq!(rows[0]["program"], "claude");
    assert_eq!(rows[0]["found"], true);
    assert_eq!(rows[0]["version"], "1.2.3");
    assert_eq!(rows[1]["id"], CUSTOM_PROVIDER);
    assert_eq!(rows[1]["program"], CUSTOM_PROGRAM);
    assert_eq!(rows[1]["found"], false);
    assert!(rows[1]["version"].is_null());
}
