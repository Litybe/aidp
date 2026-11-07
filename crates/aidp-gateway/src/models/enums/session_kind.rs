use serde::{Deserialize, Serialize};

/// The viewer-facing kind of a session, derived from its engine and origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    /// A prompt session started from the viewer.
    Free,
}
