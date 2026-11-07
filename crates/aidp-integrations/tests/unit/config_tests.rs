use super::*;
use crate::models::constants::squad::SQUAD_CONFIG_KEY;
use std::collections::BTreeMap;

const CONFIG_NAME: &str = "config.json";
const NAME_FIELD: &str = "name";
const CUSTOM_NAME: &str = "custom-squad";
const MALFORMED: &str = "not json";

/// A missing file yields the default config.
#[test]
fn missing_file_gives_the_default_config() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        load_config_from(&dir.path().join(CONFIG_NAME)),
        SquadConfig::default()
    );
}

/// A malformed file yields the default config.
#[test]
fn malformed_file_gives_the_default_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(CONFIG_NAME);
    std::fs::write(&path, MALFORMED).unwrap();
    assert_eq!(load_config_from(&path), SquadConfig::default());
}

/// The squad key overrides only the fields it names.
#[test]
fn squad_key_overrides_only_the_given_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(CONFIG_NAME);
    let document = BTreeMap::from([(
        SQUAD_CONFIG_KEY,
        BTreeMap::from([(NAME_FIELD, CUSTOM_NAME)]),
    )]);
    std::fs::write(&path, serde_json::to_string(&document).unwrap()).unwrap();
    let cfg = load_config_from(&path);
    assert_eq!(cfg.name, CUSTOM_NAME);
    assert_eq!(cfg.github_team, SquadConfig::default().github_team);
}
