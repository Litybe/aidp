use super::*;
use clap::ValueEnum;

/// Returns the clap possible value name of a variant.
fn possible_value_name(variant: EngineArg) -> String {
    variant.to_possible_value().unwrap().get_name().to_string()
}

/// Each variant's clap value name equals its engine constant.
#[test]
fn value_names_match_engine_constants() {
    assert_eq!(possible_value_name(EngineArg::Native), ENGINE_NATIVE);
    assert_eq!(possible_value_name(EngineArg::Workflow), ENGINE_WORKFLOW);
}

/// Each variant maps to the gateway engine whose wire name is its value name.
#[test]
fn engine_wire_name_matches_value_name() {
    for variant in EngineArg::value_variants() {
        assert_eq!(
            variant.into_engine().as_str(),
            possible_value_name(*variant)
        );
    }
}
