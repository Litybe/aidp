use crate::models::constants::flow_actions::{
    ACTION_BUG_CREATE, ACTION_FAIL, ACTION_GH_ISSUE_CLOSE, ACTION_GH_ISSUE_COMMENT,
    ACTION_GH_ISSUE_LABEL, ACTION_GH_MERGE, ACTION_GH_PR_COMMENT, ACTION_GH_PR_CREATE,
    ACTION_GH_PR_READY, ACTION_GIT_BRANCH, ACTION_GIT_CHECKOUT, ACTION_GIT_PUSH,
    ACTION_GIT_RESTORE, ACTION_GIT_WORKTREE, ACTION_NOOP, ACTION_TICKET_TO_TEST,
    ACTION_UNITY_REFRESH, ACTION_WORKTREE_REMOVE,
};

/// Actions every registry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinAction {
    Noop,
    Fail,
    GitWorktree,
    GitBranch,
    GitCheckout,
    GitRestore,
    GitPush,
    GhPrCreate,
    GhPrReady,
    GhMerge,
    GhIssueLabel,
    GhIssueComment,
    /// Posts a report on the pull request, where the fix stage reads the review.
    GhPrComment,
    WorktreeRemove,
    /// Dev flow done: the ticket enters the `to-test` phase (label and phase).
    TicketToTest,
    /// One compile gate of the open Unity editor, right after `git.checkout` of a test flow.
    UnityRefresh,
    /// Failed test: raises the bug issue under the ticket and makes the ticket wait for it.
    BugCreate,
    /// Closes the ticket's issue (a bug merged into its parent branch).
    GhIssueClose,
}

impl BuiltinAction {
    /// Every action the registry registers by name.
    pub const ALL: [BuiltinAction; 18] = [
        BuiltinAction::Noop,
        BuiltinAction::Fail,
        BuiltinAction::GitWorktree,
        BuiltinAction::GitBranch,
        BuiltinAction::GitCheckout,
        BuiltinAction::GitRestore,
        BuiltinAction::GitPush,
        BuiltinAction::GhPrCreate,
        BuiltinAction::GhPrReady,
        BuiltinAction::GhMerge,
        BuiltinAction::GhIssueLabel,
        BuiltinAction::GhIssueComment,
        BuiltinAction::GhPrComment,
        BuiltinAction::WorktreeRemove,
        BuiltinAction::TicketToTest,
        BuiltinAction::UnityRefresh,
        BuiltinAction::BugCreate,
        BuiltinAction::GhIssueClose,
    ];

    /// The registry name of the action.
    pub const fn as_str(self) -> &'static str {
        match self {
            BuiltinAction::Noop => ACTION_NOOP,
            BuiltinAction::Fail => ACTION_FAIL,
            BuiltinAction::GitWorktree => ACTION_GIT_WORKTREE,
            BuiltinAction::GitBranch => ACTION_GIT_BRANCH,
            BuiltinAction::GitCheckout => ACTION_GIT_CHECKOUT,
            BuiltinAction::GitRestore => ACTION_GIT_RESTORE,
            BuiltinAction::GitPush => ACTION_GIT_PUSH,
            BuiltinAction::GhPrCreate => ACTION_GH_PR_CREATE,
            BuiltinAction::GhPrReady => ACTION_GH_PR_READY,
            BuiltinAction::GhMerge => ACTION_GH_MERGE,
            BuiltinAction::GhIssueLabel => ACTION_GH_ISSUE_LABEL,
            BuiltinAction::GhIssueComment => ACTION_GH_ISSUE_COMMENT,
            BuiltinAction::GhPrComment => ACTION_GH_PR_COMMENT,
            BuiltinAction::WorktreeRemove => ACTION_WORKTREE_REMOVE,
            BuiltinAction::TicketToTest => ACTION_TICKET_TO_TEST,
            BuiltinAction::UnityRefresh => ACTION_UNITY_REFRESH,
            BuiltinAction::BugCreate => ACTION_BUG_CREATE,
            BuiltinAction::GhIssueClose => ACTION_GH_ISSUE_CLOSE,
        }
    }
}
