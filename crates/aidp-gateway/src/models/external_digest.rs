use serde::{Deserialize, Serialize};

/// The fingerprint of a result already applied to a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalDigest {
    pub job_id: String,
    pub digest: String,
}
