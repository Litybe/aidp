//! Which expression an unparsed warning refers to.

/// The kind of expression reported in an unparseable-expression warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionKind {
    When,
    WhenLoop,
    Gate,
}

impl ExpressionKind {
    /// Wire label shown in the warning text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::When => "when",
            Self::WhenLoop => "when (loop)",
            Self::Gate => "gate",
        }
    }
}
