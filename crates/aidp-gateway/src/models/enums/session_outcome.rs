use crate::models::native_reply_envelope::NativeReplyEnvelope;

/// How a session job ended.
#[derive(Debug)]
pub enum SessionOutcome {
    Completed(NativeReplyEnvelope),
    Failed(String),
    Cancelled,
}
