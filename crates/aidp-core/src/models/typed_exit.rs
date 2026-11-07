use crate::models::enums::ExitStatus;

/// An error carrying the [`ExitStatus`] the CLI should terminate with, and the
/// exact message to print (verbatim, no added prefix).
#[derive(Debug)]
pub struct TypedExit {
    pub status: ExitStatus,
    pub message: String,
}

impl std::fmt::Display for TypedExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TypedExit {}
