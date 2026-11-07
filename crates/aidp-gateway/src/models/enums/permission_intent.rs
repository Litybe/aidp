use serde::{Deserialize, Serialize};

/// What the session is allowed to do, in the host's own vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionIntent {
    AutonomousEdit,
    ReadOnly,
    Plan,
}
