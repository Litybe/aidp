use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};

/// The reply envelope the scripted provider writes to `reply.md`.
#[derive(Serialize)]
pub struct ReplyBody {
    pub status: String,
    pub summary: String,
    pub outputs: BTreeMap<String, String>,
    pub signals: Map<String, Value>,
}
