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
    None,
}

impl DoctorAction {
    /// Every allowed action, in the order the documentation lists them.
    pub const ALL: [DoctorAction; 10] = [
        Self::RetryStep,
        Self::RequeueToTest,
        Self::RequeueDev,
        Self::AbortDeadFlowAndRequeue,
        Self::UpdatePrBranch,
        Self::ReleaseUnityLock,
        Self::CreateBugTicket,
        Self::OpenHarnessPr,
        Self::RestoreTestInfra,
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
            Self::RetryStep | Self::RequeueToTest | Self::RequeueDev
        )
    }
}
