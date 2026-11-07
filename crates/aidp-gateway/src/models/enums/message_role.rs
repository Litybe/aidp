use serde::{Deserialize, Serialize};

/// The `role` field of a stream-json message written to `claude`'s stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// The message carries content authored by the user.
    User,
}

impl MessageRole {
    /// The wire value serde writes for this variant.
    pub const fn as_str(self) -> &'static str {
        match self {
            MessageRole::User => "user",
        }
    }
}
