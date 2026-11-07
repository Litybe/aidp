//! Review loop schemas: `location` on findings and the `clean` rule of `verify.json`.

use aidp_core::findings::{self, MergeInput};
use aidp_core::models::enums::diff_side::DiffSide;
use aidp_testkit::TempWorkspace;
use jsonschema::{Draft, JSONSchema};
use serde_json::Value;

const FINDINGS_SCHEMA: &str = include_str!("../../../schema/findings.schema.json");
const VERIFY_SCHEMA: &str = include_str!("../../../schema/verify.schema.json");
const FINDINGS_WITH_LOCATION: &str =
    include_str!("fixtures/review_schemas/findings_with_location.json");
const FINDINGS_LEGACY: &str = include_str!("fixtures/review_schemas/findings_legacy.json");
const VERIFY_CLEAN: &str = include_str!("fixtures/review_schemas/verify_clean.json");
const VERIFY_DIRTY: &str = include_str!("fixtures/review_schemas/verify_dirty.json");
const VERIFY_CLEAN_LIE: &str = include_str!("fixtures/review_schemas/verify_clean_lie.json");
const MERGE_INPUT_FILE: &str = "findings.json";
const MERGE_AXIS_LABEL: &str = "correctness";
const EXPECTED_PATH: &str = "src/a.rs";
const EXPECTED_LINE: u32 = 12;
const EXPECTED_START_LINE: u32 = 10;

fn is_valid(schema_text: &str, instance_text: &str) -> bool {
    let schema: Value = serde_json::from_str(schema_text).unwrap();
    let instance: Value = serde_json::from_str(instance_text).unwrap();
    let compiled = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&schema)
        .unwrap();
    compiled.is_valid(&instance)
}

#[test]
fn finding_with_location_is_valid() {
    assert!(is_valid(FINDINGS_SCHEMA, FINDINGS_WITH_LOCATION));
}

#[test]
fn finding_without_location_stays_valid() {
    assert!(is_valid(FINDINGS_SCHEMA, FINDINGS_LEGACY));
}

#[test]
fn verify_clean_is_valid() {
    assert!(is_valid(VERIFY_SCHEMA, VERIFY_CLEAN));
}

#[test]
fn verify_dirty_is_valid() {
    assert!(is_valid(VERIFY_SCHEMA, VERIFY_DIRTY));
}

#[test]
fn verify_clean_with_reopen_is_invalid() {
    assert!(!is_valid(VERIFY_SCHEMA, VERIFY_CLEAN_LIE));
}

#[test]
fn location_survives_merge() {
    let workspace = TempWorkspace::new(MERGE_AXIS_LABEL);
    let path = workspace.path().join(MERGE_INPUT_FILE);
    std::fs::write(&path, FINDINGS_WITH_LOCATION).unwrap();
    let outcome = findings::merge(&[MergeInput {
        label: MERGE_AXIS_LABEL.to_string(),
        path,
    }])
    .unwrap();
    let location = outcome.findings.findings[0].location.as_ref().unwrap();
    assert_eq!(location.path, EXPECTED_PATH);
    assert_eq!(location.line, EXPECTED_LINE);
    assert_eq!(location.start_line, Some(EXPECTED_START_LINE));
    assert_eq!(location.side, DiffSide::Right);
}
