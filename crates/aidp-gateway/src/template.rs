use serde_json::Value;
use std::collections::BTreeMap;

pub fn lookup<'a>(ctx: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = ctx;
    for k in path.split('.') {
        cur = cur.get(k)?;
    }
    Some(cur)
}

/// `{{payload.issue.key}}`, `{{event.data.sessionId}}`, `{{automation.name}}`, `{{now}}` …
pub fn render(tpl: &str, ctx: &Value) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let key = after[..end].trim();
                if let Some(v) = lookup(ctx, key) {
                    match v {
                        Value::String(s) => out.push_str(s),
                        Value::Null => {}
                        other => out.push_str(&other.to_string()),
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn render_map(
    m: &Option<BTreeMap<String, String>>,
    ctx: &Value,
) -> Option<BTreeMap<String, String>> {
    m.as_ref()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), render(v, ctx))).collect())
}
