//! `{{ path }}` placeholders in the `with` of a step, read from the same context as `when`
//! (`steps.<id>.outputs.<key>`, `inputs.*`, `ticket.*`, `settings.*`).

use crate::expr::Context;
use serde_json::Value;
use std::collections::BTreeMap;

const OPEN: &str = "{{";
const CLOSE: &str = "}}";

fn path_of(text: &str) -> Vec<String> {
    text.trim().split('.').map(str::to_string).collect()
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A string that is exactly one placeholder keeps the value's type; otherwise each
/// placeholder is replaced by its text (a missing key gives an empty text).
pub fn render_str(text: &str, context: &Context) -> Value {
    let trimmed = text.trim();
    if let Some(inner) = trimmed
        .strip_prefix(OPEN)
        .and_then(|rest| rest.strip_suffix(CLOSE))
        .filter(|inner| !inner.contains(OPEN) && !inner.contains(CLOSE))
    {
        return context.lookup(&path_of(inner));
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            out.push_str(OPEN);
            rest = after;
            continue;
        };
        out.push_str(&as_text(&context.lookup(&path_of(&after[..end]))));
        rest = &after[end + CLOSE.len()..];
    }
    out.push_str(rest);
    Value::String(out)
}

/// Renders every string inside `value`, lists and objects included.
pub fn render_value(value: &Value, context: &Context) -> Value {
    match value {
        Value::String(text) if text.contains(OPEN) => render_str(text, context),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_value(item, context))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), render_value(item, context)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The `with` of a step with its placeholders filled.
pub fn render_with(with: &BTreeMap<String, Value>, context: &Context) -> BTreeMap<String, Value> {
    with.iter()
        .map(|(key, value)| (key.clone(), render_value(value, context)))
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/template_tests.rs"]
mod tests;
