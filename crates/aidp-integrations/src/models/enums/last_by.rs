use serde::Serialize;

/// Who wrote the last comment of a review thread, relative to the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LastBy {
    Me,
    Other,
}
