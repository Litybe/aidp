use serde::{Deserialize, Serialize};

/// Per-signal toggles for pull request notifications; every signal is on by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsPrConfig {
    #[serde(default = "enabled", alias = "new_comment")]
    pub new_comment: bool,
    #[serde(default = "enabled", alias = "changes_requested")]
    pub changes_requested: bool,
    #[serde(default = "enabled", alias = "conflict")]
    pub conflict: bool,
    #[serde(default = "enabled", alias = "mergeable")]
    pub mergeable: bool,
    #[serde(default = "enabled", alias = "review_requested")]
    pub review_requested: bool,
}

/// The default of every toggle.
fn enabled() -> bool {
    true
}

impl Default for NotificationsPrConfig {
    fn default() -> Self {
        Self {
            new_comment: enabled(),
            changes_requested: enabled(),
            conflict: enabled(),
            mergeable: enabled(),
            review_requested: enabled(),
        }
    }
}
