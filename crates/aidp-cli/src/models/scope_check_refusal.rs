use serde::Serialize;

/// Result printed by `aidp scope check` when a path is refused or rules cannot load.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct ScopeCheckRefusal {
    pub ok: bool,
    pub reason: String,
}

impl ScopeCheckRefusal {
    /// Builds the refusal payload for an error message.
    pub fn new(reason: String) -> Self {
        Self { ok: false, reason }
    }
}
