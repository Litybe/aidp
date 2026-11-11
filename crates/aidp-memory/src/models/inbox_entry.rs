use serde::{Deserialize, Serialize};

/// One note waiting for review in the inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxEntry {
    /// Vault-relative path of the inbox file.
    pub path: String,
    pub title: String,
    pub domain: String,
    pub action: String,
    pub target: Option<String>,
    pub candidate_of: Option<String>,
    pub source: Option<String>,
}
