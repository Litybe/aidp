use crate::models::enums::{MemoryKind, Route};

/// Fields `memory edit` may change; `None` leaves a field as is.
#[derive(Debug, Clone, Default)]
pub struct MemoryPatch {
    pub kind: Option<MemoryKind>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub route: Option<Route>,
    pub paths: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub confidence: Option<f64>,
}
