//! Loads `templates/pr-comments.yaml` and checks its selection and resolve contract.

use std::path::PathBuf;

const TEMPLATE_ID: &str = "pr-comments";
const SELECTION_INPUT: &str = "selection";
const RESOLVE_POLICY_KEY: &str = "resolve_policy";
const DEFAULT_RESOLVE_POLICY: &str = "fixed_and_not_relevant";

fn template_path() -> PathBuf {
    aidp_testkit::fixtures::template(TEMPLATE_ID)
}

#[test]
fn it_selection_input_is_optional() {
    let pipeline = crate::pipeline::load(&template_path()).unwrap();
    let selection = pipeline.inputs.get(SELECTION_INPUT).unwrap();
    assert!(!selection.required);
}

#[test]
fn it_resolve_policy_defaults_to_fixed_and_not_relevant() {
    let pipeline = crate::pipeline::load(&template_path()).unwrap();
    let value = pipeline.settings.extra.get(RESOLVE_POLICY_KEY).unwrap();
    assert_eq!(value.as_str(), Some(DEFAULT_RESOLVE_POLICY));
}
