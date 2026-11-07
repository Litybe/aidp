//! Git arguments, GraphQL documents, reply pointers, limits and messages of the branch publish.

pub const HEAD: &str = "HEAD";
pub const ORIGIN: &str = "origin";
pub const REMOTE_REF_PREFIX: &str = "refs/remotes/origin/";
pub const BRANCH_REF_PREFIX: &str = "refs/heads/";
pub const REMOTE_BRANCH_SEPARATOR: &str = "/";
pub const SYMBOLIC_REF: &str = "symbolic-ref";
pub const QUIET_FLAG: &str = "--quiet";
pub const SHORT_FLAG: &str = "--short";
pub const CONFIG: &str = "config";
pub const GET_FLAG: &str = "--get";
pub const ORIGIN_URL_KEY: &str = "remote.origin.url";
pub const FETCH: &str = "fetch";
pub const PRUNE_FLAG: &str = "--prune";
pub const VERIFY_FLAG: &str = "--verify";
pub const MERGE_BASE: &str = "merge-base";
pub const IS_ANCESTOR_FLAG: &str = "--is-ancestor";
pub const CONFIG_FLAG: &str = "-c";
pub const LITERAL_PATHS: &str = "core.quotepath=false";
pub const DIFF: &str = "diff";
pub const NO_RENAMES_FLAG: &str = "--no-renames";
pub const NAME_STATUS_FLAG: &str = "--name-status";
pub const NUL_FLAG: &str = "-z";
pub const CAT_FILE: &str = "cat-file";
pub const SIZE_FLAG: &str = "-s";
pub const BLOB: &str = "blob";
pub const OBJECT_SEPARATOR: &str = ":";
pub const LOG: &str = "log";
pub const LAST_COMMIT_FLAG: &str = "-1";
pub const MESSAGE_FORMAT: &str = "--pretty=%B";
pub const RESET: &str = "reset";
pub const SOFT_FLAG: &str = "--soft";
pub const GRAPHQL_ENDPOINT: &str = "graphql";
pub const GIT_SUFFIX: &str = ".git";
pub const URL_SEPARATORS: [char; 2] = ['/', ':'];
pub const NUL: u8 = 0;
pub const DELETED_STATUS: char = 'D';
pub const HEADLINE_SEPARATOR: char = '\n';

/// Reads the repository node id and its default branch.
pub const REPOSITORY_QUERY: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id defaultBranchRef { name } } }";
/// Creates the branch ref on GitHub, at the commit the local branch started from.
pub const CREATE_REF_MUTATION: &str =
    "mutation($input: CreateRefInput!) { createRef(input: $input) { ref { name } } }";
/// Creates one commit GitHub signs, on top of the expected branch head.
pub const CREATE_COMMIT_MUTATION: &str = "mutation($input: CreateCommitOnBranchInput!) { createCommitOnBranch(input: $input) { commit { oid } } }";

pub const REPOSITORY_ID_POINTER: &str = "/data/repository/id";
pub const DEFAULT_BRANCH_POINTER: &str = "/data/repository/defaultBranchRef/name";
pub const COMMIT_OID_POINTER: &str = "/data/createCommitOnBranch/commit/oid";
pub const ERRORS_KEY: &str = "errors";
pub const MESSAGE_KEY: &str = "message";

pub const REPOSITORY_REQUEST_FILE: &str = "repository.json";
pub const CREATE_REF_REQUEST_FILE: &str = "create-ref.json";
pub const CREATE_COMMIT_REQUEST_FILE: &str = "create-commit.json";

/// Per-file ceiling: the whole commit travels base64 in one GraphQL request.
pub const MAX_FILE_BYTES: u64 = 45 * 1024 * 1024;
/// Ceiling of the base64 payload of one commit, all files together.
pub const MAX_TOTAL_BASE64: usize = 80 * 1024 * 1024;
pub const BYTES_PER_MEBIBYTE: u64 = 1024 * 1024;

pub const DEFAULT_MESSAGE_PREFIX: &str = "chore: update ";
pub const DETACHED_HEAD: &str = "HEAD is detached: the branch to publish is unknown";
pub const WRONG_BRANCH: &str = "the worktree is not on the branch to publish";
pub const DEFAULT_BRANCH_REFUSED: &str = "refusing to publish the repository default branch";
pub const NO_ORIGIN: &str = "remote origin";
pub const UNPARSABLE_ORIGIN: &str = "cannot read owner/name from remote origin";
pub const REMOTE_MOVED: &str =
    "has commits this worktree lacks: bring them into the worktree, then retry";
pub const NOTHING_TO_PUBLISH: &str = "nothing to publish: the branch has no change against";
pub const FILE_TOO_LARGE: &str = "is over the per-file publish limit";
pub const PAYLOAD_TOO_LARGE: &str = "the commit payload is over the publish limit while adding";
pub const NO_DEFAULT_BRANCH: &str = "GitHub reported no default branch";
pub const NO_REPOSITORY_ID: &str = "GitHub reported no repository id";
pub const NO_COMMIT_OID: &str = "GitHub reported no commit oid";
pub const GIT_FAILED: &str = "git failed";
pub const GH_FAILED: &str = "gh api graphql failed";
pub const GRAPHQL_FAILED: &str = "GitHub refused the request";
pub const ERROR_SEPARATOR: &str = "; ";
