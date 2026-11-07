use super::*;

const EVERY_BUILTIN_FLOW: &str = include_str!("../fixtures/flows/every_builtin.yaml");

/// The catalog accepts a flow that uses every builtin action and every probe.
#[test]
fn flow_catalog_accepts_every_builtin_action_and_probe() {
    let def = FlowDef::from_yaml(EVERY_BUILTIN_FLOW).unwrap();
    let report = validate(&def, &builtin_catalog(None));
    assert!(report.is_valid(), "{report:?}");
}

/// The catalog lists every action of the registry by its registry name.
#[test]
fn flow_catalog_lists_every_registry_action() {
    let catalog = builtin_catalog(None);
    assert_eq!(catalog.actions.len(), BuiltinAction::ALL.len());
    assert_eq!(catalog.wait_probes.len(), BuiltinProbe::ALL.len());
}
