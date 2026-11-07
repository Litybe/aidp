use crate::models::enums::issue_state::IssueState;
use serde::{Deserialize, Serialize};

/// One issue that blocks a mirrored ticket, keyed like a ticket (`GH-12`), with its last seen state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockerLink {
    pub key: String,
    #[serde(default)]
    pub state: IssueState,
}
