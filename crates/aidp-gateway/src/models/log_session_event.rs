use crate::models::enums::SessionEventKind;
use serde::Serialize;

/// Session bus event carrying one line of agent text output.
#[derive(Debug, Serialize)]
pub struct LogSessionEvent {
    pub kind: SessionEventKind,
    pub text: String,
}
