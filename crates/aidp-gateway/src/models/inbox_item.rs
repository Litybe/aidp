use serde::{Deserialize, Serialize};

/// A decision waiting for the user, shown in the inbox as `flow.human`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: String,
    pub step: String,
    pub iteration: u32,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub choices: Vec<String>,
    pub created_at: String,
    #[serde(default)]
    pub resolved_at: Option<String>,
}
