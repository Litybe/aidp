use serde::Serialize;

/// A request the mock saw, kept so tests can assert on what was sent.
#[derive(Clone, Serialize)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
}
