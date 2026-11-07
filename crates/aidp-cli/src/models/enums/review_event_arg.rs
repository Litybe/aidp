use aidp_integrations::models::enums::review_event::ReviewEvent;

/// Verdict accepted by `--event`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ReviewEventArg {
    /// Approve the changes.
    #[value(name = "APPROVE")]
    Approve,
    /// Block the merge until the changes are made.
    #[value(name = "REQUEST_CHANGES")]
    RequestChanges,
    /// Leave remarks without a verdict.
    #[value(name = "COMMENT")]
    Comment,
}

impl From<ReviewEventArg> for ReviewEvent {
    fn from(arg: ReviewEventArg) -> Self {
        match arg {
            ReviewEventArg::Approve => ReviewEvent::Approve,
            ReviewEventArg::RequestChanges => ReviewEvent::RequestChanges,
            ReviewEventArg::Comment => ReviewEvent::Comment,
        }
    }
}
