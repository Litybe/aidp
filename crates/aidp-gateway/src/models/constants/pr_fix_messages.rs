//! User-facing messages and text fragments of the PR fix flow.

pub const REPO_SEPARATOR: char = '/';
pub const BAD_REPO_MESSAGE: &str = "repo must be owner/name";
pub const EMPTY_SELECTION_MESSAGE: &str = "selection holds no thread and no point";
pub const NO_OPEN_THREAD_MESSAGE: &str = "every selected thread is already resolved";
pub const NO_PROJECT_MESSAGE: &str = "no registered project for repo ";
pub const NO_VAULT_MESSAGE: &str = "no vault configured";
pub const NO_SKILL_ROOT_MESSAGE: &str = "the skill root cannot be resolved";
pub const NOT_MINE_MESSAGE: &str = "the pull request is not authored by the current user";
pub const PR_CLOSED_MESSAGE: &str = "the pull request is closed";
pub const PR_MERGED_MESSAGE: &str = "the pull request is merged";
pub const BUSY_MESSAGE_PREFIX: &str = "workspace is busy: ";
pub const BECAME_BUSY_MESSAGE: &str = "workspace became busy before the session could start";
pub const SESSION_FAILED_MESSAGE: &str = "the fix session did not finish";
pub const UNKNOWN_AUTHOR: &str = "unknown";
pub const TITLE_PREFIX: &str = "Fix PR ";
