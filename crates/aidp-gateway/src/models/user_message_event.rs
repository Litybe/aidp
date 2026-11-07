use serde::Serialize;

use super::enums::SessionEventKind;

/// The session event recorded when a free user message is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserMessageEvent {
    pub kind: SessionEventKind,
    pub text: String,
    pub attachments: usize,
}
