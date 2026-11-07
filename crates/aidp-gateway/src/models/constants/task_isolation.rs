//! Git arguments, file names and messages used to run a plan task in an isolated worktree.

pub const GIT_PROGRAM: &str = "git";
pub const ISOLATION_DIR: &str = "iso";
pub const PATCH_EXTENSION: &str = ".patch";
pub const STATE_PATCH_SUFFIX: &str = ".state.patch";
pub const CONFIG_FLAG: &str = "-c";
pub const NO_AUTOCRLF_CONFIG: &str = "core.autocrlf=false";
pub const AUTHOR_NAME_CONFIG: &str = "user.name=aidp";
pub const AUTHOR_EMAIL_CONFIG: &str = "user.email=aidp@localhost";
pub const NO_SIGN_CONFIG: &str = "commit.gpgsign=false";
pub const BASE_COMMIT_MESSAGE: &str = "aidp: state of the ticket worktree when the task started";
pub const WORKTREE_ARG: &str = "worktree";
pub const ADD_ARG: &str = "add";
pub const ALL_FLAG: &str = "-A";
pub const DETACH_FLAG: &str = "--detach";
pub const REMOVE_ARG: &str = "remove";
pub const PRUNE_ARG: &str = "prune";
pub const FORCE_FLAG: &str = "--force";
pub const DIFF_ARG: &str = "diff";
pub const CACHED_FLAG: &str = "--cached";
pub const BINARY_FLAG: &str = "--binary";
pub const FULL_INDEX_FLAG: &str = "--full-index";
pub const SHOW_TOPLEVEL_FLAG: &str = "--show-toplevel";
pub const APPLY_ARG: &str = "apply";
pub const CHECK_FLAG: &str = "--check";
pub const LS_FILES_ARG: &str = "ls-files";
pub const OTHERS_FLAG: &str = "-o";
pub const EXCLUDE_STANDARD_FLAG: &str = "--exclude-standard";
pub const COMMIT_ARG: &str = "commit";
pub const NO_VERIFY_FLAG: &str = "--no-verify";
pub const ALLOW_EMPTY_FLAG: &str = "--allow-empty";
pub const QUIET_FLAG: &str = "-q";
pub const MESSAGE_FLAG: &str = "-m";
pub const PATCH_FAILED_PREFIX: &str = "patch failed: ";
pub const ERROR_PREFIX: &str = "error: ";
pub const PATH_LINE_SEPARATOR: char = ':';
pub const PATH_ERROR_SUFFIXES: [&str; 3] = [
    ": patch does not apply",
    ": already exists in working directory",
    ": does not exist in index",
];
