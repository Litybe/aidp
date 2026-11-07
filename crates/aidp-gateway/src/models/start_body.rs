use super::WorkspaceRequest;
use serde::Serialize;
use serde_json::{Map, Value};

/// Origin tag attached to a session started from the command line.
#[derive(Debug, Serialize)]
pub struct StartOrigin {
    pub kind: String,
    pub label: String,
}

/// Wire body of `POST /api/sessions` built by `aidp gateway start`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartBody {
    pub project_id: String,
    pub engine: String,
    pub template_id: Option<String>,
    pub target: Option<String>,
    pub inputs: Map<String, Value>,
    pub prompt: Option<String>,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub max_budget_usd: Option<f64>,
    pub context_from: Option<String>,
    pub workspace: Option<WorkspaceRequest>,
    pub origin: StartOrigin,
    pub auto_close: bool,
}
