use serde::{Deserialize, Serialize};

/// The known pipeline failures a diagnosis classifies an incident into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCategory {
    /// The report stage answered `needs_review` while the PR exists and the recheck is clean.
    FalseFailure,
    /// The run ended but the flow step stayed `running`.
    RunEndedFlowRunning,
    /// A Unity reimport read as a frozen editor (silent heartbeat).
    UnityReimportNotFrozen,
    /// The editor is really stuck behind a modal window.
    UnityModalDialog,
    /// A step waits for budget forever, with nothing journaled.
    BudgetWaitSilent,
    /// The worktree cap is full of blocked tickets.
    WorktreeCapSaturated,
    /// The ticket is tied to a dead flow and skipped as `owned-by-flow`.
    DeadFlowOwnership,
    /// An add/add conflict on the `.meta` of a folder several PRs created.
    MetaAddAddConflict,
    /// `gh pr update-branch --rebase` conflicts while the final state does not.
    RebaseConflictOnly,
    /// The head moved between the read and the merge.
    HeadMovedBeforeMerge,
    /// A defect of the harness itself: a patch is proposed, never applied.
    HarnessDefect,
    /// A failure of the product, not of the pipeline.
    ProductBug,
    #[default]
    #[serde(other)]
    Other,
}

impl DoctorCategory {
    /// The wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FalseFailure => "false_failure",
            Self::RunEndedFlowRunning => "run_ended_flow_running",
            Self::UnityReimportNotFrozen => "unity_reimport_not_frozen",
            Self::UnityModalDialog => "unity_modal_dialog",
            Self::BudgetWaitSilent => "budget_wait_silent",
            Self::WorktreeCapSaturated => "worktree_cap_saturated",
            Self::DeadFlowOwnership => "dead_flow_ownership",
            Self::MetaAddAddConflict => "meta_add_add_conflict",
            Self::RebaseConflictOnly => "rebase_conflict_only",
            Self::HeadMovedBeforeMerge => "head_moved_before_merge",
            Self::HarnessDefect => "harness_defect",
            Self::ProductBug => "product_bug",
            Self::Other => "other",
        }
    }
}
