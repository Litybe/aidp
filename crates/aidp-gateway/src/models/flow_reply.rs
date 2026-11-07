use serde_json::Value;

/// A status and a JSON body, handed to the HTTP layer to send.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowReply {
    pub status: u16,
    pub body: Value,
}
