//! What the doctor reaches outside the flow engine through.

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
}

/// The body of `GET /api/unity/status` for a project id.
pub type UnityProbe = Arc<dyn Fn(&str) -> Option<Value> + Send + Sync>;

/// The `doctor:` section of `TODO/config.yaml`, read at every pass.
pub type DoctorSettings = Arc<dyn Fn() -> DoctorConfig + Send + Sync>;
