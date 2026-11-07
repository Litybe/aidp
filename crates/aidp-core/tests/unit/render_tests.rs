use super::*;
use serde_json::json;

/// Compact output stays on a single line.
#[test]
fn compact_is_a_single_line() {
    let rendered = render_value(Format::JsonCompact, &json!({"a": [1, 2]})).unwrap();
    assert_eq!(rendered, r#"{"a":[1,2]}"#);
}

/// Ndjson prints one parsable line per array element.
#[test]
fn ndjson_prints_one_parsable_line_per_array_element() {
    let rendered = render_value(Format::Ndjson, &json!([{"a": 1}, {"b": [2, 3]}])).unwrap();
    assert_eq!(rendered.lines().count(), 2);
}
