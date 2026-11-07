use crate::Format;

const NDJSON_SEPARATOR: &str = "\n";

/// Renders a JSON value in the requested format.
/// `Ndjson` yields one compact line per array element; `Text` falls back to pretty JSON.
pub fn render_value(
    format: Format,
    value: &serde_json::Value,
) -> Result<String, serde_json::Error> {
    Ok(match format {
        Format::JsonCompact => serde_json::to_string(value)?,
        Format::JsonPretty | Format::Text => serde_json::to_string_pretty(value)?,
        Format::Ndjson => match value {
            serde_json::Value::Array(items) => items
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()?
                .join(NDJSON_SEPARATOR),
            other => serde_json::to_string(other)?,
        },
    })
}

#[cfg(test)]
#[path = "../tests/unit/render_tests.rs"]
mod tests;
