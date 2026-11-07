//! Facts of a flow run that fill the placeholders of its actions, and the view keys that expose them.

/// Placeholder of the pull request number.
pub const FACT_PR: &str = "pr";
/// Placeholder of the pull request URL.
pub const FACT_URL: &str = "url";
/// Placeholder of the `owner/name` repository.
pub const FACT_REPO: &str = "repo";
/// Placeholder of the ticket branch.
pub const FACT_BRANCH: &str = "branch";
/// Placeholder of the pull request head commit.
pub const FACT_SHA: &str = "sha";
/// Placeholder of the report file a comment action posts.
pub const FACT_BODY: &str = "body";
/// Placeholder of the pull request body file a pipeline stage wrote.
pub const FACT_PR_BODY: &str = "pr_body";
/// Step of the delivery flow that creates the ticket branch.
pub const STEP_BRANCH: &str = "branch";
/// Output of the branch step naming the branch.
pub const OUTPUT_BRANCH: &str = "branch";
/// Output of a native pipeline step pointing at the `pr.json` a stage wrote. The key `pr`
/// of a pipeline step is the pull request number read from `result.json`.
pub const OUTPUT_PR_FILE: &str = "pr_file";
/// Output of the `gh.pr_create` action pointing at the `pr.json` it wrote. A path when a string.
pub const OUTPUT_PR_ACTION_FILE: &str = "pr";
/// Output of a native pipeline step pointing at the `pr-body.md` a stage wrote.
pub const OUTPUT_PR_BODY_FILE: &str = "pr_body";
/// Placeholders a pull request answers.
pub const PR_FACTS: [&str; 4] = [FACT_PR, FACT_URL, FACT_REPO, FACT_SHA];
/// Longest wait for the live PR status of a run before its actions go without it.
pub const LIVE_STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// `pr.json` keys of the pull request number, by preference; a string number is accepted.
pub const PR_FILE_NUMBER_KEYS: &[&str] = &["pr_number", "number"];
/// `pr.json` keys of the pull request URL, by preference.
pub const PR_FILE_URL_KEYS: &[&str] = &["pr_url", "url", "html_url"];
/// `pr.json` keys of the `owner/name` repository, by preference.
pub const PR_FILE_REPO_KEYS: &[&str] = &["repository", "repo"];
/// `pr.json` keys of the head branch, by preference.
pub const PR_FILE_BRANCH_KEYS: &[&str] = &["branch", "head"];
/// `pr.json` keys of the head commit, by preference.
pub const PR_FILE_SHA_KEYS: &[&str] = &["commit_sha", "head_sha", "commit"];
/// `pr.json` key of a nested pull request object (`{pr: {number, url}}`).
pub const PR_FILE_NESTED_KEY: &str = "pr";
/// Marker of a pull request path in its URL, before the number.
pub const PULL_URL_MARKER: &str = "/pull/";
/// Host prefix of a GitHub URL, before `owner/name`.
pub const GITHUB_URL_HOST: &str = "github.com/";
/// Separator of the `owner/name` repository.
pub const REPO_SEPARATOR: char = '/';
/// Flow view key listing the session or native run of each pipeline effect.
pub const VIEW_SESSIONS: &str = "sessions";
/// Flow view key carrying the pull request the run opened.
pub const VIEW_PR: &str = "pr";
