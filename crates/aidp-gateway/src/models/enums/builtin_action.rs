use crate::models::constants::flow_actions::{
    ACTION_FAIL, ACTION_GH_ISSUE_COMMENT, ACTION_GH_ISSUE_LABEL, ACTION_GH_MERGE,
    ACTION_GH_PR_CREATE, ACTION_GH_PR_READY, ACTION_GIT_BRANCH, ACTION_GIT_PUSH,
    ACTION_GIT_WORKTREE, ACTION_NOOP, ACTION_WORKTREE_REMOVE,
};

/// Actions every registry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinAction {
    Noop,
    Fail,
    GitWorktree,
    GitBranch,
    GitPush,
    GhPrCreate,
    GhPrReady,
    GhMerge,
    GhIssueLabel,
    GhIssueComment,
    WorktreeRemove,
}

impl BuiltinAction {
    /// Every action the registry registers by name.
    pub const ALL: [BuiltinAction; 11] = [
        BuiltinAction::Noop,
        BuiltinAction::Fail,
        BuiltinAction::GitWorktree,
        BuiltinAction::GitBranch,
        BuiltinAction::GitPush,
        BuiltinAction::GhPrCreate,
        BuiltinAction::GhPrReady,
        BuiltinAction::GhMerge,
        BuiltinAction::GhIssueLabel,
        BuiltinAction::GhIssueComment,
        BuiltinAction::WorktreeRemove,
    ];

    /// The registry name of the action.
    pub const fn as_str(self) -> &'static str {
        match self {
            BuiltinAction::Noop => ACTION_NOOP,
            BuiltinAction::Fail => ACTION_FAIL,
            BuiltinAction::GitWorktree => ACTION_GIT_WORKTREE,
            BuiltinAction::GitBranch => ACTION_GIT_BRANCH,
            BuiltinAction::GitPush => ACTION_GIT_PUSH,
            BuiltinAction::GhPrCreate => ACTION_GH_PR_CREATE,
            BuiltinAction::GhPrReady => ACTION_GH_PR_READY,
            BuiltinAction::GhMerge => ACTION_GH_MERGE,
            BuiltinAction::GhIssueLabel => ACTION_GH_ISSUE_LABEL,
            BuiltinAction::GhIssueComment => ACTION_GH_ISSUE_COMMENT,
            BuiltinAction::WorktreeRemove => ACTION_WORKTREE_REMOVE,
        }
    }
}
