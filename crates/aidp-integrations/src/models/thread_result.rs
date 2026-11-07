//! Outcome of resolving or unresolving one review thread.

use serde::Serialize;

/// One entry per requested thread id, in request order.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResult {
    pub id: String,
    pub ok: bool,
    pub error: Option<String>,
}
