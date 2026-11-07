use super::*;
use serde::Deserialize;

const RECIPE: &str =
    include_str!("../../../../fixtures/skill/templates/recipes/fix-pr-comments.md");
const FIXTURE: &str = include_str!("../fixtures/pr_fix/selection.json");

#[derive(Deserialize)]
struct Fixture {
    selected_ids: Vec<String>,
    input: PrFixRecipeInput,
}

fn selected_input() -> PrFixRecipeInput {
    let fixture: Fixture = serde_json::from_str(FIXTURE).unwrap();
    let mut input = fixture.input;
    input
        .threads
        .retain(|thread| fixture.selected_ids.contains(&thread.id));
    input
}

#[test]
fn pr_fix_recipe_renders_only_selected_threads() {
    let prompt = render(RECIPE, &selected_input());
    assert!(prompt.contains("PRRT_two") && prompt.contains("BODY_TWO"));
    assert!(prompt.contains("PRRT_four") && prompt.contains("BODY_FOUR"));
    assert!(!prompt.contains("PRRT_one") && !prompt.contains("BODY_ONE"));
    assert!(!prompt.contains("PRRT_three") && !prompt.contains("BODY_THREE"));
    assert!(prompt.contains("POINT_ONE") && prompt.contains("NOTE_TEXT"));
    assert!(prompt.contains("acme/widgets") && prompt.contains("feat/widgets"));
}

#[test]
fn pr_fix_recipe_escapes_fences_and_placeholders() {
    let prompt = render(RECIPE, &selected_input());
    assert!(prompt.contains("use ''' fences { {note}} here"));
    assert!(!prompt.contains("use ``` fences"));
    assert!(!prompt.contains("{{"));
}

#[test]
fn pr_fix_recipe_states_push_and_no_question_rules() {
    let prompt = render(RECIPE, &selected_input());
    assert!(prompt.contains("Push only to the branch `feat/widgets`"));
    assert!(prompt.contains("Never use `--force`"));
    assert!(prompt.contains("Never ask a question"));
    assert!(prompt.contains(PR_FIX_SUMMARY_PREFIX));
}

#[test]
fn pr_fix_recipe_parse_summary_takes_last_line() {
    let tail = format!(
        "x\n{PR_FIX_SUMMARY_PREFIX}{{\"fixed\":[\"a\"]}}\nmore\n{PR_FIX_SUMMARY_PREFIX}{{\"fixed\":[\"b\"]}}\n"
    );
    let summary = parse_summary(&tail).unwrap();
    assert_eq!(summary["fixed"][0], "b");
}

#[test]
fn pr_fix_recipe_parse_summary_none_when_missing_or_malformed() {
    assert!(parse_summary("no summary here").is_none());
    assert!(parse_summary(&format!("{PR_FIX_SUMMARY_PREFIX}{{not json")).is_none());
}
