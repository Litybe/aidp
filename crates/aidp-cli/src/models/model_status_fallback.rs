use aidp_memory::ModelStatus;
use serde::Serialize;

/// What `aidp memory model status` prints when the daemon does not answer: the files and the vectors on disk.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusFallback {
    pub daemon_running: bool,
    #[serde(flatten)]
    pub status: ModelStatus,
}
