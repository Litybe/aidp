//! Run state: init, load, persist, render.

mod cost;
mod fold;
mod gates;
mod init;
mod layout;
mod reconcile;
mod render;
mod stage_knowledge;

use crate::pipeline::Placement;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::collections::BTreeMap;

pub use crate::models::run_state::{
    GateDecision, LoopState, RunCost, RunPorts, RunState, StageState, StageStatus, StageUsage,
};

pub use crate::models::enums::envelope_status::EnvelopeStatus;
pub use crate::models::enums::run_status::RunStatus;
pub use crate::models::enums::workflow_status::WorkflowStatus;
pub use cost::{ComplexityBucket, UsageSource};
pub use fold::{fold_workflow_result, plan_accounting_anomalies};
pub use gates::{
    anchor_not_persisted, approved_gates, overall_status, parse_auto_approve_gates,
    parse_resume_anchor, read_resume_anchor, record_resume, record_resume_with,
    resolve_approve_gates, stage_complete_noop, stage_skip, GateDecisionKind, ResumeAnchor,
    WORKFLOW_RESULT_FILE,
};
pub use init::{
    ensure_knowledge_snapshot, init, init_with, init_with_mode, new_run_id, NATIVE_RUN_MODE,
};
pub use layout::{
    feature_key, load_in, run_dir_for_in, save, save_state, INPUT_JIRA_KEY, INPUT_TARGET,
    RUN_STATE_FILE,
};
pub use reconcile::{carry_over_task_outcomes, reconcile, reconcile_with, record_plan_schedules};
pub use render::{render_progress, render_summary};
pub use stage_knowledge::{ensure_stage_knowledge, stage_context_request};

impl RunState {
    /// The shared per-feature directory (`…/<feature>/`, in the vault under `Pipelines/<repo>/` or
    /// in the legacy `.aidp/pipelines/`).
    pub fn ticket_root(&self) -> PathBuf {
        self.run_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.run_dir.clone())
    }

    /// Directory holding `stage_name`'s artifacts, per its [`Placement`]: knowledge stages resolve
    /// to the per-project knowledge dir, research stages to the shared ticket root.
    pub fn stage_dir(&self, stage_name: &str, placement: Placement) -> PathBuf {
        match placement {
            Placement::Knowledge => self.knowledge_dir(),
            Placement::KnowledgeFeature => self.knowledge_features_dir(),
            Placement::Ticket => self.ticket_root().join(stage_segment(stage_name)),
            Placement::Run => self.run_dir.join(stage_segment(stage_name)),
        }
    }

    /// The per-project knowledge directory.
    pub fn knowledge_dir(&self) -> PathBuf {
        self.knowledge_root
            .clone()
            .unwrap_or_else(|| self.ticket_root())
    }

    /// The per-feature layer of the knowledge base — `<knowledge_dir>/Features`.
    pub fn knowledge_features_dir(&self) -> PathBuf {
        crate::knowledge::features_dir(&self.knowledge_dir())
    }
}

/// Fallback segment for a stage name that cannot be sanitized. Never
/// `_`-prefixed: that prefix is reserved for generated index notes.
const INVALID_STAGE_SEGMENT: &str = "invalid-stage";

/// The stage-name rule `^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$`, char-based.
/// Every declared stage name, `<composite>__<stage>` included, satisfies it.
fn is_safe_stage_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// One safe path segment for `stage_name`, disambiguated when the name is invalid.
fn stage_segment(stage_name: &str) -> String {
    if is_safe_stage_name(stage_name) {
        return stage_name.to_string();
    }
    match crate::vault::sanitize_segment(stage_name) {
        Some(s) if !s.starts_with('_') && is_safe_segment(&s) => s,
        _ => format!("{INVALID_STAGE_SEGMENT}-{}", short_hash(stage_name)),
    }
}

/// Short, stable, filesystem-safe hash of an arbitrary raw stage name, used
/// only to disambiguate the [`INVALID_STAGE_SEGMENT`] fallback.
fn short_hash(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:08x}", (hasher.finish() & 0xffff_ffff) as u32)
}

/// Defense in depth on whatever `sanitize_segment` returned.
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && !s.contains("..") && !s.contains(['/', '\\', ':'])
}

#[cfg(test)]
#[path = "../../tests/unit/run_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/run_status_tests.rs"]
mod run_status_tests;
