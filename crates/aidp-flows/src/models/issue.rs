use crate::models::enums::IssueCode;
use serde::{Deserialize, Serialize};

/// One validation finding. `message` is the human sentence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub code: IssueCode,
    pub step: Option<String>,
    pub message: String,
}
