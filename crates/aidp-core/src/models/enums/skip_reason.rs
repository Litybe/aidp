use serde::{Deserialize, Serialize};

/// Why a stage is absent from the pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The stage's own `when:` is statically false.
    StaticWhen,
    /// An enclosing composite's `when:` is statically false.
    CompositeWhen,
}
