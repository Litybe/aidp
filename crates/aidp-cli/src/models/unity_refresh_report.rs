//! What `flow unity-refresh` prints: the verdict of one compile gate of the open editor.

use serde::Serialize;
use serde_json::Value;

/// The outputs of the `unity.refresh` action. `skipped` marks a refresh that had nothing to do
/// (no Unity project, no editor open); `blocked` an editor that never answered.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct UnityRefreshReport {
    pub request_id: Option<String>,
    pub ok: bool,
    pub blocked: bool,
    pub skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub errors: Vec<Value>,
    pub warnings: u64,
    pub duration_ms: u64,
    pub message: String,
    pub heartbeat: Value,
}
