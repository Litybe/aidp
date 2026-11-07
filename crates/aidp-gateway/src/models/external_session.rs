use serde::Serialize;

/// A Claude Code transcript found under the user's projects directory.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSession {
    pub id: String,
    pub first_prompt: String,
    pub modified: String,
}
