//! Names, defaults and messages of the squad pull request inbox.

pub const CONFIG_FILE_DIR: &str = ".aidp";
pub const CONFIG_FILE_NAME: &str = "config.json";
pub const SQUAD_CONFIG_KEY: &str = "squad";

pub const DEFAULT_SQUAD_NAME: &str = "squad";
/// No GitHub team by default: the inbox reads `extra_repos` and the registered repositories.
pub const DEFAULT_GITHUB_TEAM: &str = "";
pub const BOT_DEPENDABOT: &str = "dependabot[bot]";
pub const BOT_RENOVATE: &str = "renovate[bot]";
pub const BOT_LOGIN_SUFFIX: &str = "[bot]";

/// Repositories per listing query: the listing reads scalars only, so it stays cheap.
pub const LIST_BATCH_SIZE: usize = 5;
/// Pull requests per detail query, each read in full.
pub const DETAIL_BATCH_SIZE: usize = 20;
/// `gh` calls run at once: a first read of hundreds of pull requests takes seconds, not minutes.
pub const PARALLEL_CALLS: usize = 6;
/// Gateway statuses GitHub answers when a query times out: the batch is split in halves,
/// and a single repository or pull request is retried once.
/// The last entry is how `gh` reports a response GitHub cut short.
pub const TRANSIENT_HTTP_STATUSES: [&str; 4] = [
    "HTTP 502",
    "HTTP 503",
    "HTTP 504",
    "unexpected end of JSON input",
];
pub const REPOS_CACHE_FILE: &str = "squad-repos.json";
pub const REPOS_CACHE_TTL_MS: i64 = 24 * 60 * 60 * 1000;
pub const SLUG_SEPARATOR: char = '/';
pub const SLUG_EXTRA_CHARS: [char; 3] = ['-', '_', '.'];

pub const GH_API: &str = "api";
pub const GH_GRAPHQL: &str = "graphql";
pub const GH_USER_PATH: &str = "user";
pub const GH_JQ_FLAG: &str = "--jq";
pub const GH_PAGINATE_FLAG: &str = "--paginate";
pub const GH_FIELD_FLAG: &str = "-f";
pub const GH_TYPED_FIELD_FLAG: &str = "-F";
pub const GH_USER_LOGIN_JQ: &str = ".login";
pub const GH_TEAM_REPOS_JQ: &str = ".[] | select(.archived | not) | .full_name";
pub const TEAM_PATH_ORGS: &str = "orgs";
pub const TEAM_PATH_TEAMS: &str = "teams";
pub const TEAM_PATH_REPOS: &str = "repos";

pub const VARIABLE_ME: &str = "me";
pub const VARIABLE_OWNER_PREFIX: &str = "o";
pub const VARIABLE_NAME_PREFIX: &str = "n";
pub const VARIABLE_NUMBER_PREFIX: &str = "k";
pub const ALIAS_PREFIX: &str = "r";
pub const QUERY_ARG_PREFIX: &str = "query=";

/// Fields the listing reads: what tells a changed pull request apart, and the states
/// (checks, mergeability, review decision) that move without bumping `updatedAt`.
pub const LIST_FIELDS: &str = r"
number updatedAt headRefOid isDraft mergeable reviewDecision
commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
";

/// Fields read for every new or changed pull request; `$me` filters the reviews.
pub const PR_FIELDS: &str = r"
number title url isDraft createdAt updatedAt baseRefName headRefName headRefOid
mergeable reviewDecision
author { login }
reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } ... on Team { slug } } } }
reviews(first: 20, author: $me) { nodes { state submittedAt author { login } commit { oid } } }
commits(last: 1) { totalCount nodes { commit { statusCheckRollup { state } } } }
reviewThreads(first: 100) { nodes { isResolved } }
comments { totalCount }
labels(first: 20) { nodes { name } }
recentComments: comments(last: 20) { nodes { id createdAt body author { login } } }
recentThreads: reviewThreads(last: 20) { nodes { comments(last: 20) { nodes { id createdAt body author { login } } } } }
";
pub const PRS_PER_REPO: u32 = 50;

pub const PTR_DATA: &str = "/data";
pub const PTR_PR_NODES: &str = "/pullRequests/nodes";
pub const PTR_PULL_REQUEST: &str = "/pullRequest";
pub const PTR_AUTHOR_LOGIN: &str = "/author/login";
pub const PTR_REQUEST_NODES: &str = "/reviewRequests/nodes";
pub const PTR_REQUESTED_LOGIN: &str = "/requestedReviewer/login";
pub const PTR_REQUESTED_SLUG: &str = "/requestedReviewer/slug";
pub const PTR_REVIEW_NODES: &str = "/reviews/nodes";
pub const PTR_REVIEW_COMMIT: &str = "/commit/oid";
pub const PTR_COMMITS_TOTAL: &str = "/commits/totalCount";
pub const PTR_ROLLUP_STATE: &str = "/commits/nodes/0/commit/statusCheckRollup/state";
pub const PTR_THREAD_NODES: &str = "/reviewThreads/nodes";
pub const PTR_COMMENTS_TOTAL: &str = "/comments/totalCount";
pub const PTR_RECENT_COMMENT_NODES: &str = "/recentComments/nodes";
pub const PTR_RECENT_THREAD_NODES: &str = "/recentThreads/nodes";
pub const PTR_THREAD_COMMENT_NODES: &str = "/comments/nodes";
pub const PTR_LABEL_NODES: &str = "/labels/nodes";
pub const KEY_NUMBER: &str = "number";
pub const KEY_TITLE: &str = "title";
pub const KEY_URL: &str = "url";
pub const KEY_IS_DRAFT: &str = "isDraft";
pub const KEY_CREATED_AT: &str = "createdAt";
pub const KEY_UPDATED_AT: &str = "updatedAt";
pub const KEY_BASE_REF: &str = "baseRefName";
pub const KEY_HEAD_REF: &str = "headRefName";
pub const KEY_HEAD_OID: &str = "headRefOid";
pub const KEY_MERGEABLE: &str = "mergeable";
pub const KEY_REVIEW_DECISION: &str = "reviewDecision";
pub const KEY_STATE: &str = "state";
pub const KEY_SUBMITTED_AT: &str = "submittedAt";
pub const KEY_IS_RESOLVED: &str = "isResolved";
pub const KEY_ID: &str = "id";
pub const KEY_BODY: &str = "body";
pub const KEY_NAME: &str = "name";

pub const MESSAGE_TEAM_FAILED: &str = "team repositories unavailable, using extra and registered repositories; run gh auth refresh -s read:org";
pub const MESSAGE_TEAM_INVALID: &str = "github_team must be <org>/<team-slug>";
pub const MESSAGE_REPO_MISSING: &str = "repository not returned by GraphQL";
pub const MESSAGE_INVALID_REPO: &str = "repository slug must be <owner>/<name>";
pub const MESSAGE_NO_LOGIN: &str = "gh returned no login";
pub const MESSAGE_NO_DATA: &str = "GraphQL response has no data";
pub const MESSAGE_KEPT_PREVIOUS: &str = "showing the last known state";
pub const MESSAGE_CALL_PANICKED: &str = "gh call panicked";
