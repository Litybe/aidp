use std::fmt;

/// Why the dispatcher did not launch a candidate this tick, or gave up on a
/// launch attempt already in flight. Distinct from `crate::plan::SkipReason`,
/// which classifies why `plan` never selected a candidate for launch in the
/// first place; this enum instead names the failure modes of actually
/// executing a launch the plan already selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The candidate's project already has a session running.
    ProjectBusy,
    /// The ticket note no longer exists when re-read before launch.
    Vanished,
    /// The ticket was edited after the plan was built, so the plan is stale.
    ChangedSincePlan,
    /// The candidate's service has no routed project.
    NeedsRouting,
    /// The queued transition is not legal from the ticket's current status.
    IllegalTransition,
    /// Saving the ticket as queued failed.
    SaveFailed,
    /// Writing the pipeline inputs file failed.
    InputsFileFailed,
    /// `SessionManager::start` returned an error.
    StartFailed,
    /// The session was requeued behind a race with another launch.
    QueueRace,
    /// A native run already occupies the candidate's workspace.
    WorkspaceBusy,
    /// `launch: flow` is set but no flow engine is attached.
    FlowUnavailable,
    /// The ticket's delivery flow is active, so the flow owns it.
    FlowActive,
}

impl SkipReason {
    /// The stable wire slug for this reason, unchanged across the daemon's
    /// state file, its HTTP API and the docs.
    pub const fn as_str(&self) -> &'static str {
        match self {
            SkipReason::ProjectBusy => "project-busy",
            SkipReason::Vanished => "vanished",
            SkipReason::ChangedSincePlan => "changed-since-plan",
            SkipReason::NeedsRouting => "needs-routing",
            SkipReason::IllegalTransition => "illegal-transition",
            SkipReason::SaveFailed => "save-failed",
            SkipReason::InputsFileFailed => "inputs-file-failed",
            SkipReason::StartFailed => "start-failed",
            SkipReason::QueueRace => "queue-race",
            SkipReason::WorkspaceBusy => "workspace-busy",
            SkipReason::FlowUnavailable => "flow-unavailable",
            SkipReason::FlowActive => "owned-by-flow",
        }
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
