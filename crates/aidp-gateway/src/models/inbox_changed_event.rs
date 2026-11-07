use serde::Serialize;

/// Payload of the inbox-changed bus event: empty, the type alone says what happened.
#[derive(Debug, Serialize)]
pub struct InboxChangedEvent {}
