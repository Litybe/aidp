use crate::models::constants::flow_external::DUPLICATE_KEY;
use serde_json::Value;

/// The answer to a posted result: the run view, and whether the post replayed an applied result.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalReply {
    pub view: Value,
    pub duplicate: bool,
}

impl ExternalReply {
    /// The response body: the run view, with `duplicate: true` for a replay.
    pub fn into_body(self) -> Value {
        let mut body = self.view;
        if self.duplicate {
            if let Some(map) = body.as_object_mut() {
                map.insert(DUPLICATE_KEY.to_string(), Value::Bool(true));
            }
        }
        body
    }
}
