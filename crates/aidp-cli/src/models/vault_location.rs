use serde::Serialize;

/// Vault root and how it was resolved, as reported by `aidp run start`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct VaultLocation<'a> {
    pub root: String,
    pub source: &'a str,
}
