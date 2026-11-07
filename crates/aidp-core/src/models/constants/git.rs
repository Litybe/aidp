//! Program name and arguments passed to the `git` binary.

pub const GIT_PROGRAM: &str = "git";
pub const GIT_DIR_FLAG: &str = "-C";
pub const REV_PARSE: &str = "rev-parse";
pub const HEAD_REF: &str = "HEAD";
pub const ABBREV_REF_FLAG: &str = "--abbrev-ref";
pub const IS_INSIDE_WORK_TREE_FLAG: &str = "--is-inside-work-tree";
pub const LOG: &str = "log";
pub const LOG_FORMAT_FLAG: &str = "--format=%H%x1f%s%x1f%aI";
pub const NAME_ONLY_FLAG: &str = "--name-only";
pub const LAST_ONE_FLAG: &str = "-1";
pub const COMMITTER_DATE_FORMAT_FLAG: &str = "--format=%cI";
pub const PATH_SEPARATOR_ARG: &str = "--";
pub const WORKTREE: &str = "worktree";
pub const LIST: &str = "list";
pub const PORCELAIN_FLAG: &str = "--porcelain";
pub const WORKTREE_LINE_PREFIX: &str = "worktree ";
pub const RECORDING_GIT_POISONED: &str = "RecordingGit mutex must not be poisoned";
