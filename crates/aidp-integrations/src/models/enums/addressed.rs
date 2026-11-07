use serde::Serialize;

/// Advisory signal that a review thread was already addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Addressed {
    Direct,
    Indirect,
    No,
}
