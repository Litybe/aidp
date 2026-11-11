/// How a proposed note relates to the notes already in the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuplicateVerdict {
    /// The same note: only its evidence grows. Holds the vault-relative path.
    Same(String),
    /// Close to a note: a human decides. Holds the vault-relative path.
    Close(String),
    /// Nothing comparable.
    Fresh,
}
