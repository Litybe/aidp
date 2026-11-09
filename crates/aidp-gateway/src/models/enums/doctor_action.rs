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
    None,
}

impl DoctorAction {
    /// Every allowed action, in the order the documentation lists them.
    pub const ALL: [DoctorAction; 7] = [
        Self::RetryStep,
        Self::RequeueToTest,
        Self::RequeueDev,
        Self::AbortDeadFlowAndRequeue,
        Self::UpdatePrBranch,
        Self::ReleaseUnityLock,
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
            Self::None => "none",
        }
    }

    /// The allowed action named `text`; `None` for any name outside the list.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.as_str() == text.trim())
    }
}
