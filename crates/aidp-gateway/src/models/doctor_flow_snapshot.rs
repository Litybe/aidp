use aidp_flows::FlowState;
use std::collections::BTreeMap;

/// A flow run as the doctor reads it from disk, with the live state of its pipeline runs.
#[derive(Debug, Clone)]
pub struct DoctorFlowSnapshot {
    pub repo: String,
    pub state: FlowState,
    /// Effect id (`<step>:<iteration>:start_pipeline`) to native run or session id.
    pub sessions: BTreeMap<String, String>,
    /// The run's actions and probes are executed by its caller, not by the daemon.
    pub external: bool,
    /// An action of the run is executing in a worker thread.
    pub action_inflight: bool,
}
