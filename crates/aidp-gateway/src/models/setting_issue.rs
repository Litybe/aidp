use serde::{Deserialize, Serialize};

/// One refused value of a settings update: where it sits and why it was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingIssue {
    pub path: String,
    pub message: String,
}

impl SettingIssue {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> SettingIssue {
        SettingIssue {
            path: path.into(),
            message: message.into(),
        }
    }
}
