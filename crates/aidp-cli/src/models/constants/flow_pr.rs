//! File name, PR view fields and messages of `flow pr-create`.

pub const PR_FILE_NAME: &str = "pr.json";
pub const OPEN_PR_STATE: &str = "OPEN";
pub const PR_STATE_FIELD: &str = "state";
pub const PR_NUMBER_FIELD: &str = "number";
pub const PR_URL_FIELD: &str = "url";
pub const PR_REPO_FIELD: &str = "repo";
pub const PR_HEAD_SHA_FIELD: &str = "head_sha";
pub const TITLE_SEPARATOR: &str = ": ";
pub const NO_BRANCH_MESSAGE: &str = "the worktree is on no branch; pass --branch";
pub const NO_OPENED_PR_MESSAGE: &str = "the pull request was created but cannot be read back";
pub const FRONT_MATTER_FENCE: &str = "---";
pub const WIKILINK_OPEN: &str = "[[";
pub const WIKILINK_CLOSE: &str = "]]";
pub const WIKILINK_ALIAS_SEPARATOR: char = '|';
