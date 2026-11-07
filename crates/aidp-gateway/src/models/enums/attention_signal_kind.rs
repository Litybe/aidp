use serde::{Deserialize, Serialize};

/// A pull request event worth the viewer's attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionSignalKind {
    NewComment,
    ChangesRequested,
    Conflict,
    Mergeable,
    ReviewRequested,
}

impl AttentionSignalKind {
    /// The wire value, also used in fingerprints.
    pub const fn as_str(self) -> &'static str {
        match self {
            AttentionSignalKind::NewComment => "new_comment",
            AttentionSignalKind::ChangesRequested => "changes_requested",
            AttentionSignalKind::Conflict => "conflict",
            AttentionSignalKind::Mergeable => "mergeable",
            AttentionSignalKind::ReviewRequested => "review_requested",
        }
    }
}
