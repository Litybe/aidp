//! What the doctor reaches outside the flow engine through.

use crate::models::constants::doctor::NO_MODAL_CHANNEL;
use crate::models::native_run_start::StartNativeRun;
use crate::models::native_run_view::NativeRunView;
use crate::native_runs::NativeRunManager;
use aidp_tickets::config::DoctorConfig;
use serde_json::Value;
use std::sync::Arc;

/// The native runs: the paused ones are incidents, the investigations are native runs too.
pub trait DoctorRuns: Send + Sync {
    fn list(&self) -> Vec<NativeRunView>;
    fn get(&self, run_id: &str) -> Option<NativeRunView>;
    fn start(&self, request: StartNativeRun) -> anyhow::Result<String>;
    /// Resumes a paused run: its failed stages run again, the finished ones are kept.
    fn resume(&self, run_id: &str) -> anyhow::Result<()>;
    /// Cancels the live sessions of a run; how many were cancelled.
    fn cancel_sessions(&self, run_id: &str) -> anyhow::Result<usize>;
    /// Resumes a paused run from `stage`: that stage and the later ones run again.
    fn resume_from_stage(&self, run_id: &str, stage: &str) -> anyhow::Result<()>;
    /// Sets the model the remaining stages of a run use.
    fn set_model(&self, run_id: &str, model: &str) -> anyhow::Result<()>;
}

/// The daemon's native run executor.
pub struct NativeDoctorRuns(pub Arc<NativeRunManager>);

impl DoctorRuns for NativeDoctorRuns {
    fn list(&self) -> Vec<NativeRunView> {
        self.0.list()
    }

    fn get(&self, run_id: &str) -> Option<NativeRunView> {
        self.0.get(run_id)
    }

    fn start(&self, request: StartNativeRun) -> anyhow::Result<String> {
        self.0.start(request)
    }

    fn resume(&self, run_id: &str) -> anyhow::Result<()> {
        self.0.retry_failed(run_id)
    }

    fn cancel_sessions(&self, run_id: &str) -> anyhow::Result<usize> {
        self.0.cancel_sessions(run_id)
    }

    fn resume_from_stage(&self, run_id: &str, stage: &str) -> anyhow::Result<()> {
        self.0.retry_from(run_id, stage)
    }

    fn set_model(&self, run_id: &str, model: &str) -> anyhow::Result<()> {
        self.0.set_model(run_id, model)
    }
}

/// The channel to the modal windows of a Unity editor.
pub trait UnityModals: Send + Sync {
    /// Closes the modal window that blocks the editor of project `project_id`; the reason when it cannot.
    fn dismiss(&self, project_id: &str) -> Result<String, String>;
}

/// The default channel: the playtest package exposes none, so no modal is ever closed.
pub struct NoUnityModals;

impl UnityModals for NoUnityModals {
    fn dismiss(&self, _project_id: &str) -> Result<String, String> {
        Err(NO_MODAL_CHANNEL.to_string())
    }
}

/// The body of `GET /api/unity/status` for a project id.
pub type UnityProbe = Arc<dyn Fn(&str) -> Option<Value> + Send + Sync>;

/// The `doctor:` section of `TODO/config.yaml`, read at every pass.
pub type DoctorSettings = Arc<dyn Fn() -> DoctorConfig + Send + Sync>;

/// Local clone of a harness repository (`owner/name`), where a patch is cut from `origin/main`.
pub type HarnessRepos = Arc<dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync>;
