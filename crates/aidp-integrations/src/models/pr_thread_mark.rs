use serde::{Deserialize, Serialize};

/// Mark left on one review thread handed to a fix session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrThreadMark {
    pub fix_session: String,
    pub at: String,
}
