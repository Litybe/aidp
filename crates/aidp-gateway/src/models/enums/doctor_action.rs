use serde::{Deserialize, Serialize};

/// The actions the daemon accepts from a diagnosis. Anything else is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorAction {
    RetryStep,
    RequeueToTest,
    RequeueDev,
    /// Only when the flow is terminal.
    AbortDeadFlowAndRequeue,
    /// `gh pr update-branch` by merge, never by rebase.
    UpdatePrBranch,
    /// Only when the holder is terminal.
    ReleaseUnityLock,
    /// Raises a bug issue under the ticket, through `aidp flow bug-create` (same marker, same cap).
    CreateBugTicket,
    /// Opens a pull request on the harness repository from a patch; never merges.
    OpenHarnessPr,
    /// Puts back from `origin/main` the test-infrastructure files a branch broke, on the branch, then reruns its test.
    RestoreTestInfra,
    /// Closes the modal window that blocks the Unity editor of the project, then resumes the step.
    DismissUnityModal,
    /// Resumes a Unity compile gate step with a longer wait, for that run only.
    ExtendUnityWait,
    /// Counts a report or verdict step as passed when the pull request exists and the recheck is clean.
    MarkStepPassed,
    /// Reads again the state of a step whose run ended, without resuming the run.
    ReconcileStep,
    /// Evaluates again the budget guard of a step that waits for budget.
    NudgeBudget,
    /// Removes the worktrees of the flows that are terminal or blocked for long.
    PruneWorktrees,
    /// Merges `origin/main` into the pull request branch, keeping main's `.meta` files on add/add conflicts.
    ResolveMetaConflict,
    /// Reads the pull request head again and runs the `gh.merge` step again, never forcing.
    RetryMergeStep,
    /// Cancels the orphan session of a stuck step, then reconciles it.
    KillStaleSession,
    /// Resumes a paused run from a named stage, discarding that stage and the later ones.
    RetryFromStage,
    /// Resumes a run with the fallback model of the settings after the provider stayed overloaded.
    RetryWithFallback,
    /// Skips the current step when its template marks it `optional`.
    SkipOptionalStep,
    /// Pushes a clean copy of the pull request branch, rebuilt from `origin/main`, to `doctor/rebuild/<incident>`.
    RebuildBranch,
    None,
}

impl DoctorAction {
    /// Every allowed action, in the order the documentation lists them.
    pub const ALL: [DoctorAction; 23] = [
        Self::RetryStep,
        Self::RequeueToTest,
        Self::RequeueDev,
        Self::AbortDeadFlowAndRequeue,
        Self::UpdatePrBranch,
        Self::ReleaseUnityLock,
        Self::CreateBugTicket,
        Self::OpenHarnessPr,
        Self::RestoreTestInfra,
        Self::DismissUnityModal,
        Self::ExtendUnityWait,
        Self::MarkStepPassed,
        Self::ReconcileStep,
        Self::NudgeBudget,
        Self::PruneWorktrees,
        Self::ResolveMetaConflict,
        Self::RetryMergeStep,
        Self::KillStaleSession,
        Self::RetryFromStage,
        Self::RetryWithFallback,
        Self::SkipOptionalStep,
        Self::RebuildBranch,
        Self::None,
    ];

    /// The wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RetryStep => "retry_step",
            Self::RequeueToTest => "requeue_to_test",
            Self::RequeueDev => "requeue_dev",
            Self::AbortDeadFlowAndRequeue => "abort_dead_flow_and_requeue",
            Self::UpdatePrBranch => "update_pr_branch",
            Self::ReleaseUnityLock => "release_unity_lock",
            Self::CreateBugTicket => "create_bug_ticket",
            Self::OpenHarnessPr => "open_harness_pr",
            Self::RestoreTestInfra => "restore_test_infra",
            Self::DismissUnityModal => "dismiss_unity_modal",
            Self::ExtendUnityWait => "extend_unity_wait",
            Self::MarkStepPassed => "mark_step_passed",
            Self::ReconcileStep => "reconcile_step",
            Self::NudgeBudget => "nudge_budget",
            Self::PruneWorktrees => "prune_worktrees",
            Self::ResolveMetaConflict => "resolve_meta_conflict",
            Self::RetryMergeStep => "retry_merge_step",
            Self::KillStaleSession => "kill_stale_session",
            Self::RetryFromStage => "retry_from_stage",
            Self::RetryWithFallback => "retry_with_fallback",
            Self::SkipOptionalStep => "skip_optional_step",
            Self::RebuildBranch => "rebuild_branch",
            Self::None => "none",
        }
    }

    /// The allowed action named `text`; `None` for any name outside the list.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.as_str() == text.trim())
    }

    /// True for the actions that undo themselves or change nothing lasting: a lower confidence
    /// is enough to run them.
    pub const fn is_reversible(self) -> bool {
        matches!(
            self,
            Self::RetryStep
                | Self::RequeueToTest
                | Self::RequeueDev
                | Self::ExtendUnityWait
                | Self::ReconcileStep
                | Self::NudgeBudget
        )
    }
}
