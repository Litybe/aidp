use serde::{Deserialize, Serialize};

/// Why a step ended the way it did. Stable machine-read value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepReason {
    WhenFalse,
    SkipOn,
    ManualSkip,
    ActionSkipped,
    ActionWaiting,
    ExpressionError,
    TechnicalError,
    BusinessError,
    Orphaned,
    Budget,
    MaxIterations,
    RunStatus,
    PipelineFailed,
    Aborted,
    Rejected,
    /// A pipeline step waits for a human because its ticket reached its token ceiling.
    TicketBudget,
    /// A top-level step before the entry a run was started from by hand.
    Forced,
}

impl StepReason {
    /// The wire value stored in the step state.
    pub fn as_str(self) -> &'static str {
        match self {
            StepReason::WhenFalse => "when-false",
            StepReason::SkipOn => "skip-on",
            StepReason::ManualSkip => "manual-skip",
            StepReason::ActionSkipped => "action-skipped",
            StepReason::ActionWaiting => "action-waiting",
            StepReason::ExpressionError => "expression-error",
            StepReason::TechnicalError => "technical-error",
            StepReason::BusinessError => "business-error",
            StepReason::Orphaned => "orphaned",
            StepReason::Budget => "budget",
            StepReason::MaxIterations => "max-iterations",
            StepReason::RunStatus => "run-status",
            StepReason::PipelineFailed => "pipeline-failed",
            StepReason::Aborted => "aborted",
            StepReason::Rejected => "rejected",
            StepReason::TicketBudget => "ticket-budget",
            StepReason::Forced => "forced",
        }
    }
}
