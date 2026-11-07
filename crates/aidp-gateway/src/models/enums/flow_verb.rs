/// Verbs of the flow routes, run-level and step-level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowVerb {
    Pause,
    Resume,
    Abort,
    Retry,
    Skip,
    Approve,
    Reject,
    Click,
    Verdict,
}

impl FlowVerb {
    const ALL: [FlowVerb; 9] = [
        FlowVerb::Pause,
        FlowVerb::Resume,
        FlowVerb::Abort,
        FlowVerb::Retry,
        FlowVerb::Skip,
        FlowVerb::Approve,
        FlowVerb::Reject,
        FlowVerb::Click,
        FlowVerb::Verdict,
    ];

    /// The path segment of the verb.
    pub const fn as_str(self) -> &'static str {
        match self {
            FlowVerb::Pause => "pause",
            FlowVerb::Resume => "resume",
            FlowVerb::Abort => "abort",
            FlowVerb::Retry => "retry",
            FlowVerb::Skip => "skip",
            FlowVerb::Approve => "approve",
            FlowVerb::Reject => "reject",
            FlowVerb::Click => "click",
            FlowVerb::Verdict => "verdict",
        }
    }

    /// Parses a path segment; `None` for an unknown verb.
    pub fn parse(segment: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|verb| verb.as_str() == segment)
    }
}
