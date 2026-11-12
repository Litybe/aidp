use serde::{Deserialize, Serialize};

/// Machine-read kind of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCode {
    DuplicateStepId,
    MissingUse,
    UnknownUse,
    UnknownStepRef,
    BadExpression,
    LoopUnbounded,
    UnknownPipelineTemplate,
    CatalogUnreadable,
    BadResource,
    BadEntry,
}
