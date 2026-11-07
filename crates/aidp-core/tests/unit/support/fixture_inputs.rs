use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The fixture inputs of template `id`, or an empty object when it has none.
pub(crate) fn fixture_inputs(id: &str) -> Value {
    let raw = std::fs::read_to_string(aidp_testkit::fixtures::template_inputs())
        .expect("read template inputs fixture");
    let all: BTreeMap<String, Value> = serde_json::from_str(&raw).unwrap();
    all.get(id)
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()))
}
