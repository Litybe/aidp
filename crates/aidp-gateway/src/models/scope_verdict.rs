/// Outcome of a post-stage scope check: what broke the scope and what was undone.
/// A non-empty `violations` means the stage must be marked failed.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScopeVerdict {
    pub violations: Vec<String>,
    pub restored: Vec<String>,
}

impl ScopeVerdict {
    /// True when the stage stayed inside its declared scope.
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}
