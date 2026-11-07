use crate::models::constants::flow_git::{
    FEATURE_BRANCH_PREFIX, FEATURE_COMMIT_TYPE, FIX_BRANCH_PREFIX, FIX_COMMIT_TYPE,
};
use aidp_tickets::model::IssueType;

/// The branch family of a ticket: `fix/` for a bug, `feat/` for everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Fix,
    Feat,
}

impl BranchKind {
    pub const fn prefix(self) -> &'static str {
        match self {
            BranchKind::Fix => FIX_BRANCH_PREFIX,
            BranchKind::Feat => FEATURE_BRANCH_PREFIX,
        }
    }

    /// The Conventional Commits type of the family, as a pull request title starts.
    pub const fn commit_type(self) -> &'static str {
        match self {
            BranchKind::Fix => FIX_COMMIT_TYPE,
            BranchKind::Feat => FEATURE_COMMIT_TYPE,
        }
    }
}

impl From<IssueType> for BranchKind {
    fn from(issue_type: IssueType) -> Self {
        if issue_type == IssueType::Bug {
            BranchKind::Fix
        } else {
            BranchKind::Feat
        }
    }
}
