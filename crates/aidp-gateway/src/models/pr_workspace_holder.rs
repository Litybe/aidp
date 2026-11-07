use crate::models::constants::pr_workspace_holder::{
    HOLDER_RUN_INFIX, HOLDER_SESSION_INFIX, HOLDER_STAGE_PREFIX,
};
use serde::{Deserialize, Serialize};

/// The stage or session currently holding a pull request branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrWorkspaceHolder {
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// ISO 8601 UTC instant the branch was taken.
    pub acquired_at: String,
}

impl PrWorkspaceHolder {
    /// The holder as the busy message names it.
    pub fn describe(&self) -> String {
        let mut text = format!("{HOLDER_STAGE_PREFIX}{}", self.stage);
        if let Some(run_id) = &self.run_id {
            text.push_str(&format!("{HOLDER_RUN_INFIX}{run_id}"));
        }
        if let Some(session_id) = &self.session_id {
            text.push_str(&format!("{HOLDER_SESSION_INFIX}{session_id}"));
        }
        text
    }
}
