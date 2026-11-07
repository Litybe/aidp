use serde::{Deserialize, Serialize};

/// Why a launch waits in the queue; serialized as the kebab-case wire value.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QueueReason {
    WorkspaceBusy,
    ReadOnlyCeiling,
}
