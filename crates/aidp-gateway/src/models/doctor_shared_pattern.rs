use serde::{Deserialize, Serialize};

/// The same normalized error found in the test reports of several tickets over the last 24 hours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorSharedPattern {
    /// Stable key: a known signature name, or the normalized line.
    pub key: String,
    /// One real line carrying the pattern.
    pub sample: String,
    /// Tickets whose reports carry it (a bug counts for its parent).
    pub tickets: Vec<String>,
    /// Native runs whose reports carry it.
    pub runs: Vec<String>,
}
