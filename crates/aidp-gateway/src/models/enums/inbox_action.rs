/// Inbox action offered on an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxAction {
    Decide,
    Open,
}

impl InboxAction {
    /// The wire spelling of the action.
    pub const fn as_str(self) -> &'static str {
        match self {
            InboxAction::Decide => "decide",
            InboxAction::Open => "open",
        }
    }
}
