use serde::{Deserialize, Serialize};

/// Which reply schema a stage's sub-agent answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplySchemaKind {
    Standard,
    StandardWithSkills,
    Reviewer,
    ReviewerWithSkills,
}
