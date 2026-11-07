use super::*;
use serde_json::json;

fn context() -> Context {
    Context::new(json!({
        "inputs": {"name": "hero"},
        "steps": {"review": {"outputs": {"pr": 42, "verdict": "done", "ok": true}}},
    }))
}

#[test]
fn a_lone_placeholder_keeps_the_type() {
    assert_eq!(
        render_str("{{ steps.review.outputs.pr }}", &context()),
        json!(42)
    );
    assert_eq!(
        render_str("{{steps.review.outputs.ok}}", &context()),
        json!(true)
    );
}

#[test]
fn placeholders_inside_text_become_text() {
    let text = "PR #{{ steps.review.outputs.pr }} for {{ inputs.name }}: {{ steps.review.outputs.verdict }}";
    assert_eq!(render_str(text, &context()), json!("PR #42 for hero: done"));
}

#[test]
fn a_missing_key_is_empty_and_an_open_brace_is_kept() {
    assert_eq!(render_str("a{{ inputs.none }}b", &context()), json!("ab"));
    assert_eq!(render_str("a {{ b", &context()), json!("a {{ b"));
}

#[test]
fn with_renders_nested_values() {
    let with = BTreeMap::from([
        ("body".to_string(), json!("see {{ inputs.name }}")),
        (
            "list".to_string(),
            json!([{"n": "{{ steps.review.outputs.pr }}"}]),
        ),
        ("plain".to_string(), json!(3)),
    ]);
    let out = render_with(&with, &context());
    assert_eq!(out["body"], json!("see hero"));
    assert_eq!(out["list"], json!([{"n": 42}]));
    assert_eq!(out["plain"], json!(3));
}
