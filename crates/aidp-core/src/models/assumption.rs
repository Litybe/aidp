//! A choice an autonomous stage took in place of asking a question.

use serde::{Deserialize, Serialize};

/// One journaled autonomous choice, with its reason and the alternative it rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assumption {
    #[serde(default)]
    pub stage: String,
    pub choice: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_alternative: Option<String>,
    #[serde(default)]
    pub auto_answered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
}

#[cfg(test)]
#[path = "../../tests/unit/assumption_tests.rs"]
mod tests;
