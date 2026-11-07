use serde_json::Value;

/// The values an expression reads: one JSON object whose roots are
/// `steps`, `settings`, `inputs` and `ticket`.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    root: Value,
}

impl Context {
    /// Wraps a JSON root object.
    pub fn new(root: Value) -> Self {
        Context { root }
    }

    /// Follows `segments` from the root; a missing key reads as null.
    pub fn lookup(&self, segments: &[String]) -> Value {
        let mut current = &self.root;
        for segment in segments {
            let next = match current {
                Value::Object(map) => map.get(segment),
                Value::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
                _ => None,
            };
            match next {
                Some(value) => current = value,
                None => return Value::Null,
            }
        }
        current.clone()
    }
}
