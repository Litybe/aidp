use serde::{Deserialize, Serialize};

/// What an external job asks the caller to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalJobKind {
    Action,
    Probe,
    /// An agent session of a pipeline run, launched by the host.
    Agent,
}
