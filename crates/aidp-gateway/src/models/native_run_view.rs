use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::native_run_start::StartNativeRun;
use crate::models::native_stage_view::NativeStageView;
use aidp_core::models::assumption::Assumption;
use aidp_core::models::escalation::Escalation;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A native run as the API shows it and as the daemon persists it beside `run-state.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRunView {
    pub run_id: String,
    pub template_id: String,
    pub status: NativeRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub workspace: PathBuf,
    pub run_dir: PathBuf,
    /// The launch request, kept so a restart relaunches with the same settings.
    pub request: StartNativeRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_gate: Option<String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// Stage names grouped in dependency layers, as `run-state.json` records them.
    #[serde(default)]
    pub plan: Vec<Vec<String>>,
    pub stages: Vec<NativeStageView>,
    /// True when the run never asks: stages journal assumptions instead.
    #[serde(default)]
    pub autonomous: bool,
    /// Choices autonomous stages took in place of a question.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assumptions: Vec<Assumption>,
    /// Decisions stages escalated to a human.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub escalations: Vec<Escalation>,
}

impl NativeRunView {
    /// The row named `name`.
    pub fn stage(&self, name: &str) -> Option<&NativeStageView> {
        self.stages.iter().find(|row| row.name == name)
    }

    /// The row named `name`, appended as pending when absent (plan tasks and shards).
    pub fn stage_or_insert(&mut self, name: &str) -> &mut NativeStageView {
        if let Some(position) = self.stages.iter().position(|row| row.name == name) {
            return &mut self.stages[position];
        }
        self.stages.push(NativeStageView::pending(name));
        let last = self.stages.len() - 1;
        &mut self.stages[last]
    }

    /// Every session id of the run, in launch order.
    pub fn session_ids(&self) -> Vec<String> {
        self.stages
            .iter()
            .flat_map(|row| row.session_ids.iter().cloned())
            .collect()
    }
}
