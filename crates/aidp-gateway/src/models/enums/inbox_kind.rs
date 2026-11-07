/// Inbox kind of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InboxKind {
    /// A human step waiting for a decision.
    #[serde(rename = "flow.human")]
    Human,
    /// A pull request event worth the viewer's attention.
    #[serde(rename = "pr.attention")]
    Attention,
    /// A native run waiting at a gate.
    #[serde(rename = "gate")]
    Gate,
}

impl InboxKind {
    /// The wire spelling of the kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            InboxKind::Human => "flow.human",
            InboxKind::Attention => "pr.attention",
            InboxKind::Gate => "gate",
        }
    }
}
