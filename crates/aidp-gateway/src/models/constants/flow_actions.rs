//! Registry names, `aidp` verbs, flags and `with` keys of the flow actions.

pub const ACTION_NOOP: &str = "noop";
pub const ACTION_FAIL: &str = "fail";
pub const ACTION_GIT_WORKTREE: &str = "git.worktree";
pub const ACTION_GIT_BRANCH: &str = "git.branch";
pub const ACTION_GIT_PUSH: &str = "git.push";
pub const ACTION_GH_PR_CREATE: &str = "gh.pr_create";
pub const ACTION_GH_PR_READY: &str = "gh.pr_ready";
pub const ACTION_GH_MERGE: &str = "gh.merge";
pub const ACTION_GH_ISSUE_LABEL: &str = "gh.issue_label";
pub const ACTION_GH_ISSUE_COMMENT: &str = "gh.issue_comment";
pub const ACTION_WORKTREE_REMOVE: &str = "worktree.remove";

pub const VERB_FLOW: &str = "flow";
pub const VERB_FLOW_GIT_BRANCH: &str = "git-branch";
pub const VERB_FLOW_GIT_PUSH: &str = "git-push";
pub const VERB_FLOW_PR_CREATE: &str = "pr-create";
pub const VERB_GATEWAY: &str = "gateway";
pub const VERB_GATEWAY_WORKTREES: &str = "worktrees";
pub const VERB_WORKTREES_ENSURE: &str = "ensure";
pub const VERB_WORKTREES_REMOVE: &str = "remove";
pub const VERB_TICKET: &str = "ticket";
pub const VERB_TICKET_LABEL: &str = "label";
pub const VERB_TICKET_COMMENT: &str = "comment";
pub const VERB_GH: &str = "gh";
pub const VERB_GH_PR_READY: &str = "pr-ready";
pub const VERB_GH_PR_MERGE: &str = "pr-merge";

pub const FLAG_BODY_FILE: &str = "--body-file";
pub const FLAG_EXPECTED_HEAD_SHA: &str = "--expected-head-sha";
pub const FLAG_BRANCH: &str = "--branch";
pub const FLAG_DRAFT: &str = "--draft";
pub const FLAG_COST_FOOTER: &str = "--cost-footer";

pub const PLACEHOLDER_KEY_ARG: &str = "{key}";
pub const PLACEHOLDER_PR_ARG: &str = "{pr}";
pub const PLACEHOLDER_BRANCH_ARG: &str = "{branch}";
pub const PLACEHOLDER_SHA_ARG: &str = "{sha}";
pub const PLACEHOLDER_PR_BODY_ARG: &str = "{pr_body}";
pub const PLACEHOLDER_STATE_ARG: &str = "{state}";
pub const PLACEHOLDER_BODY_FILE_ARG: &str = "{body}";

pub const WITH_TO: &str = "to";
pub const WITH_STATUS: &str = "status";
pub const WITH_COST_FOOTER: &str = "cost_footer";
pub const WITH_BODY: &str = "body";

pub const OUTPUT_SKIPPED: &str = "skipped";
pub const OUTPUT_REASON: &str = "reason";
pub const OUTPUT_WAITING_EXTERNAL: &str = "waiting_external";

pub const GIT_PUSH_TIMEOUT_SECONDS: u64 = 15 * 60;

pub const LOCAL_KEY_SEPARATOR: &str = "-";
