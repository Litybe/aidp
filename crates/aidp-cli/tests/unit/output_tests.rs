use super::*;
use serde_json::json;

/// Pretty and text both render an indented object.
#[test]
fn pretty_and_text_are_indented() {
    for format in [Format::JsonPretty, Format::Text] {
        let rendered = render_value(format, &json!({"a": 1})).unwrap();
        assert!(rendered.contains("\n  \"a\""), "{rendered}");
    }
}

/// A non-array value prints on one line under Ndjson.
#[test]
fn ndjson_prints_a_non_array_value_on_one_line() {
    let rendered = render_value(Format::Ndjson, &json!({"a": {"b": 1}})).unwrap();
    assert_eq!(rendered, r#"{"a":{"b":1}}"#);
}
