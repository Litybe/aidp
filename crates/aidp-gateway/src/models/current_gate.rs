use serde::Serialize;
use serde_json::Value;

/// `currentGate` payload of a paused run. `workflow_run_id` and
/// `resume_command` are only present when the resume anchor was never
/// persisted: `resume_command` records it from the saved
/// `workflow-result.json`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentGate {
    pub stage: Option<String>,
    pub anchor_persisted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
}

impl CurrentGate {
    /// A gate whose resume anchor is recorded in `run-state.json`.
    pub fn persisted(stage: &str) -> Self {
        CurrentGate {
            stage: Some(stage.to_string()),
            anchor_persisted: true,
            workflow_run_id: None,
            resume_command: None,
        }
    }

    /// A gate whose resume anchor was never persisted.
    pub fn unpersisted(
        stage: Option<String>,
        workflow_run_id: Option<String>,
        resume_command: String,
    ) -> Self {
        CurrentGate {
            stage,
            anchor_persisted: false,
            workflow_run_id,
            resume_command: Some(resume_command),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
