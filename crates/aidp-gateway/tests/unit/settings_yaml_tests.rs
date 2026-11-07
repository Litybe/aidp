use super::*;
use serde_yaml::Value;

const CONFIG: &str = include_str!("../fixtures/settings/config.yaml");
const ROUTING: &str = include_str!("../fixtures/settings/routing.yaml");
const PER_TICKET_ART: [&str; 3] = ["budget", "per_ticket", "art"];
const SLOTS: [&str; 2] = ["dispatcher", "slots"];
const NIGHT_ENABLED: [&str; 3] = ["dispatcher", "night", "enabled"];
const STOP_PERCENT: [&str; 2] = ["dispatcher", "stop_at_7d_percent"];
const PIXELLAB_RESERVE: [&str; 3] = ["budget", "pixellab", "min_reserve"];

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

fn set(parts: &[&str], value: Value) -> LeafEdit {
    LeafEdit {
        path: path(parts),
        value: Some(value),
    }
}

fn number(value: u64) -> Value {
    Value::Number(value.into())
}

fn parse(text: &str) -> Value {
    serde_yaml::from_str(text).expect("valid yaml")
}

#[test]
fn replacing_a_leaf_keeps_comments_and_the_comment_column() {
    let patched = patch_leaves(CONFIG, &[set(&SLOTS, number(3))]).unwrap();

    assert!(patched.contains("  slots: 3                      # tickets in-progress at once"));
    assert_eq!(patched.lines().count(), CONFIG.lines().count());
    assert!(patched.contains("# aidp ticket dispatcher configuration"));
}

#[test]
fn a_missing_leaf_is_inserted_under_its_existing_parent() {
    let patched = patch_leaves(CONFIG, &[set(&PIXELLAB_RESERVE, number(5))]).unwrap();

    let document = parse(&patched);
    assert_eq!(document["budget"]["pixellab"]["min_reserve"], number(5));
    assert_eq!(document["budget"]["max_utilization_5h"], parse("0.80"));
    assert_eq!(
        document["dispatcher_extra_note"],
        Value::String("kept".into())
    );
}

#[test]
fn a_missing_chain_is_created_with_the_right_indent() {
    let patched = patch_leaves(
        CONFIG,
        &[
            set(&NIGHT_ENABLED, Value::Bool(true)),
            set(&PER_TICKET_ART, number(900)),
        ],
    )
    .unwrap();

    let document = parse(&patched);
    assert_eq!(
        document["dispatcher"]["night"]["enabled"],
        Value::Bool(true)
    );
    assert_eq!(document["budget"]["per_ticket"]["art"], number(900));
    assert!(patched.contains("  night:\n    enabled: true"));
}

#[test]
fn removing_a_leaf_drops_its_line_only() {
    let with_stop = patch_leaves(CONFIG, &[set(&STOP_PERCENT, number(85))]).unwrap();
    let removed = patch_leaves(
        &with_stop,
        &[LeafEdit {
            path: path(&STOP_PERCENT),
            value: None,
        }],
    )
    .unwrap();

    assert_eq!(removed, CONFIG);
}

#[test]
fn a_flow_style_parent_asks_for_a_rewrite() {
    let flow = "budget: { max_utilization_5h: 0.8 }\n";

    assert_eq!(
        patch_leaves(flow, &[set(&PIXELLAB_RESERVE, number(1))]),
        None
    );
}

#[test]
fn text_amounts_are_written_plain_and_other_texts_quoted() {
    assert_eq!(render_scalar(&Value::String("3M".into())), "3M");
    assert_eq!(render_scalar(&Value::String("1.5m".into())), "1.5m");
    assert_eq!(render_scalar(&Value::String("22:00".into())), "\"22:00\"");
    assert_eq!(render_scalar(&Value::Bool(true)), "true");
}

#[test]
fn replacing_a_section_keeps_the_header_and_the_other_sections() {
    let replaced = replace_section(ROUTING, "defaults", Some("defaults:\n  track: deep\n"));

    assert!(replaced.starts_with("# TODO/routing.yaml for PixelHood"));
    assert_eq!(
        parse(&replaced)["defaults"]["track"],
        Value::String("deep".into())
    );
    assert_eq!(parse(&replaced)["repos"], parse(ROUTING)["repos"]);
}

#[test]
fn replacing_a_block_sequence_written_at_the_key_indent_takes_all_its_items() {
    let text = "head: 1\nrules:\n- a: 1\n  b: 2\n# note\n- a: 3\ntail: 2\n";

    let replaced = replace_section(text, "rules", Some("rules: []\n"));

    assert_eq!(replaced, "head: 1\nrules: []\ntail: 2\n");
}

#[test]
fn a_missing_section_is_appended_after_a_blank_line() {
    let appended = replace_section(
        "a: 1\n",
        "models",
        Some("models:\n  code:\n    review: sonnet\n"),
    );

    assert_eq!(appended, "a: 1\n\nmodels:\n  code:\n    review: sonnet\n");
}

#[test]
fn dropping_a_section_removes_the_whole_block() {
    let dropped = replace_section(
        "a: 1\nmodels:\n  code:\n    review: opus\nb: 2\n",
        "models",
        None,
    );

    assert_eq!(dropped, "a: 1\nb: 2\n");
}

#[test]
fn windows_line_endings_are_kept() {
    let patched = patch_leaves("a:\r\n  b: 1\r\n", &[set(&["a", "b"], number(2))]).unwrap();

    assert_eq!(patched, "a:\r\n  b: 2\r\n");
}

#[test]
fn apply_to_value_creates_and_removes_nested_keys() {
    let mut document = parse("a: 1\n");

    apply_to_value(&mut document, &path(&["x", "y"]), Some(&number(2)));
    assert_eq!(document["x"]["y"], number(2));
    apply_to_value(&mut document, &path(&["x", "y"]), None);
    assert_eq!(document["x"], parse("{}"));
}
