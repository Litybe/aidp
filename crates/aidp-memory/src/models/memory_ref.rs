use crate::models::enums::RefType;
use serde::{Deserialize, Serialize};

/// A pointer from a memory to the issue, pull request or run that taught it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRef {
    #[serde(rename = "type")]
    pub ref_type: RefType,
    pub id: String,
}
