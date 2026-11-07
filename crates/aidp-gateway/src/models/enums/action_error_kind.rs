/// Failure family of an action error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionErrorKind {
    Business,
    Technical,
}

impl ActionErrorKind {
    /// The `with.kind` value selecting this family.
    pub const fn as_str(self) -> &'static str {
        match self {
            ActionErrorKind::Business => "business",
            ActionErrorKind::Technical => "technical",
        }
    }
}
