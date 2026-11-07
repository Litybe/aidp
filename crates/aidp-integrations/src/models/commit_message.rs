//! The message of a commit GitHub creates.

use serde::Serialize;

/// The first line, then the rest of the message when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitMessage {
    pub headline: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}
