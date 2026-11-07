use super::*;

const BASIC: &str = include_str!("../fixtures/basic.yaml");
const LOOP_MAX: &str = include_str!("../fixtures/loop_max.yaml");
const INVALID_DUPLICATE: &str = include_str!("../fixtures/invalid_duplicate.yaml");
const INVALID_LOOP: &str = include_str!("../fixtures/invalid_loop.yaml");

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn catalog() -> Catalog {
    Catalog {
        actions: names(&["noop", "fail"]),
        wait_probes: names(&["timer"]),
        pipelines: Some(names(&["dev"])),
    }
}

fn codes(issues: &[Issue]) -> Vec<IssueCode> {
    issues.iter().map(|issue| issue.code).collect()
}

fn def_of(text: &str) -> FlowDef {
    FlowDef::from_yaml(text).unwrap()
}

#[test]
fn accepts_the_basic_and_loop_fixtures() {
    assert!(validate(&def_of(BASIC), &catalog()).is_valid());
    assert!(validate(&def_of(LOOP_MAX), &catalog()).is_valid());
}

#[test]
fn refuses_duplicate_ids_including_loop_bodies() {
    let report = validate(&def_of(INVALID_DUPLICATE), &catalog());

    let duplicates: Vec<&Issue> = report
        .errors
        .iter()
        .filter(|issue| issue.code == IssueCode::DuplicateStepId)
        .collect();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0].step.as_deref(), Some("a"));
}

#[test]
fn refuses_unknown_use() {
    let report = validate(&def_of(INVALID_DUPLICATE), &catalog());

    assert!(codes(&report.errors).contains(&IssueCode::UnknownUse));
}

#[test]
fn refuses_when_pointing_to_an_unknown_step() {
    let report = validate(&def_of(INVALID_DUPLICATE), &catalog());

    assert!(codes(&report.errors).contains(&IssueCode::UnknownStepRef));
}

#[test]
fn refuses_until_pointing_to_an_unknown_step() {
    let mut def = def_of(LOOP_MAX);
    def.steps[0].until = Some("steps.ghost.outputs.green == true".to_string());

    let report = validate(&def, &catalog());

    assert!(codes(&report.errors).contains(&IssueCode::UnknownStepRef));
}

#[test]
fn accepts_an_output_key_unknown_until_runtime_on_a_known_step() {
    let mut def = def_of(LOOP_MAX);
    def.steps[0].until = Some("steps.check.outputs.verdict == 'done'".to_string());

    let report = validate(&def, &catalog());

    assert!(report.is_valid(), "{:?}", report.errors);
}

#[test]
fn refuses_loop_without_max_and_without_human_step() {
    let report = validate(&def_of(INVALID_LOOP), &catalog());

    assert!(codes(&report.errors).contains(&IssueCode::LoopUnbounded));
}

#[test]
fn accepts_loop_without_max_when_a_human_step_decides() {
    let mut def = def_of(INVALID_LOOP);
    def.steps[0].body.push(Step::new("ask", StepKind::Human));

    let report = validate(&def, &catalog());

    assert!(!codes(&report.errors).contains(&IssueCode::LoopUnbounded));
}

#[test]
fn refuses_unknown_pipeline_template() {
    let report = validate(&def_of(INVALID_LOOP), &catalog());

    assert!(codes(&report.errors).contains(&IssueCode::UnknownPipelineTemplate));
}

#[test]
fn unreadable_catalog_only_warns_once() {
    let mut unreadable = catalog();
    unreadable.pipelines = None;
    let mut def = def_of(INVALID_LOOP);
    def.steps[0].max = Some(1);
    let mut second = Step::new("second", StepKind::Pipeline);
    second.use_ = Some("other".to_string());
    def.steps.push(second);

    let report = validate(&def, &unreadable);

    assert!(report.is_valid());
    assert_eq!(codes(&report.warnings), vec![IssueCode::CatalogUnreadable]);
}

#[test]
fn refuses_unparsable_expression() {
    let mut def = def_of(BASIC);
    def.steps[3].when = Some("steps.review ==".to_string());

    let report = validate(&def, &catalog());

    assert!(codes(&report.errors).contains(&IssueCode::BadExpression));
}

#[test]
fn refuses_missing_use_for_action_wait_and_pipeline() {
    let mut def = def_of(BASIC);
    def.steps = vec![
        Step::new("a", StepKind::Action),
        Step::new("w", StepKind::Wait),
        Step::new("p", StepKind::Pipeline),
    ];

    let report = validate(&def, &catalog());

    assert_eq!(codes(&report.errors), vec![IssueCode::MissingUse; 3]);
}

#[test]
fn block_use_is_known_without_the_registry() {
    let mut def = def_of(BASIC);
    let mut reference = Step::new("ref", StepKind::Action);
    reference.use_ = Some(CI_BLOCK_NAME.to_string());
    def.steps.push(reference);

    let report = validate(&def, &Catalog::default());

    assert!(!report
        .errors
        .iter()
        .any(|issue| issue.step.as_deref() == Some("ref")));
}

const CI_BLOCK_NAME: &str = "ci";

#[test]
fn duplicate_scope_names_keep_their_message_text() {
    assert_eq!(DuplicateScope::Flow.as_str(), "flow");
    assert_eq!(DuplicateScope::Block("ci").as_str(), "ci");
}

#[test]
fn accepts_the_unity_editor_resource_on_a_pipeline_step() {
    let def = def_of(
        "id: f
version: 1
steps:
  - { id: a, kind: pipeline, use: dev, resource: unity-editor }
",
    );

    assert!(validate(&def, &catalog()).is_valid());
    assert_eq!(def.steps[0].resource.as_deref(), Some("unity-editor"));
}

#[test]
fn refuses_an_unknown_resource() {
    let def = def_of(
        "id: f
version: 1
steps:
  - { id: a, kind: pipeline, use: dev, resource: gpu }
",
    );

    let report = validate(&def, &catalog());

    assert_eq!(codes(&report.errors), vec![IssueCode::BadResource]);
    assert!(report.errors[0].message.contains("unity-editor"));
}

#[test]
fn refuses_a_resource_on_a_non_pipeline_step() {
    let def = def_of(
        "id: f
version: 1
steps:
  - { id: a, kind: action, use: noop, resource: unity-editor }
",
    );

    let report = validate(&def, &catalog());

    assert_eq!(codes(&report.errors), vec![IssueCode::BadResource]);
}
