use serde::{Deserialize, Serialize};

/// Captured command streams and the same exit code used by the aidp process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedCommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
