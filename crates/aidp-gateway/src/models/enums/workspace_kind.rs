use std::str::FromStr;

use serde::{Deserialize, Serialize};

const PROJECT: &str = "project";
const TICKET: &str = "ticket";
const SCRATCH: &str = "scratch";
const PATH: &str = "path";
const PR_WORKTREE: &str = "prworktree";

/// Whether a session runs in the project checkout, a ticket worktree, a scratch worktree or a free path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceKind {
    Project,
    #[serde(alias = "worktree")]
    Ticket,
    Scratch,
    Path,
    PrWorktree,
}

impl FromStr for WorkspaceKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            PROJECT => Ok(Self::Project),
            TICKET => Ok(Self::Ticket),
            SCRATCH => Ok(Self::Scratch),
            PATH => Ok(Self::Path),
            PR_WORKTREE => Ok(Self::PrWorktree),
            other => Err(format!("unknown workspace kind {other:?}")),
        }
    }
}
