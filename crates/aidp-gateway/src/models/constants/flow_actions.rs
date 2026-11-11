//! Registry names, `aidp` verbs, flags and `with` keys of the flow actions.

pub const ACTION_NOOP: &str = "noop";
pub const ACTION_FAIL: &str = "fail";
pub const ACTION_GIT_WORKTREE: &str = "git.worktree";
pub const ACTION_GIT_BRANCH: &str = "git.branch";
pub const ACTION_GIT_CHECKOUT: &str = "git.checkout";
pub const ACTION_GIT_RESTORE: &str = "git.restore";
pub const ACTION_GIT_PUSH: &str = "git.push";
pub const ACTION_GH_PR_CREATE: &str = "gh.pr_create";
pub const ACTION_GH_PR_READY: &str = "gh.pr_ready";
pub const ACTION_GH_MERGE: &str = "gh.merge";
pub const ACTION_GH_ISSUE_LABEL: &str = "gh.issue_label";
pub const ACTION_GH_ISSUE_COMMENT: &str = "gh.issue_comment";
pub const ACTION_WORKTREE_REMOVE: &str = "worktree.remove";
pub const ACTION_TICKET_TO_TEST: &str = "ticket.to_test";
/// Raises the bug issue of a failed test, as a sub-issue of the ticket under test.
pub const ACTION_BUG_CREATE: &str = "bug.create";
/// Closes the GitHub issue of the ticket (a bug merged into its parent branch, not into `main`).
pub const ACTION_GH_ISSUE_CLOSE: &str = "gh.issue_close";
/// One compile gate of the open Unity editor after `git.checkout`, before the test stage.
pub const ACTION_UNITY_REFRESH: &str = "unity.refresh";
/// State label (and phase) the `ticket.to_test` action sets.
pub const STATE_TO_TEST: &str = "to-test";

pub const VERB_FLOW: &str = "flow";
pub const VERB_FLOW_GIT_BRANCH: &str = "git-branch";
pub const VERB_FLOW_GIT_CHECKOUT: &str = "git-checkout";
pub const VERB_FLOW_GIT_RESTORE: &str = "git-restore";
pub const VERB_FLOW_GIT_PUSH: &str = "git-push";
pub const VERB_FLOW_PR_CREATE: &str = "pr-create";
pub const VERB_FLOW_UNITY_REFRESH: &str = "unity-refresh";
pub const VERB_FLOW_BUG_CREATE: &str = "bug-create";
/// Re-arms the bug cap of a ticket (`aidp flow bug-rearm KEY`).
pub const VERB_FLOW_BUG_REARM: &str = "bug-rearm";
pub const VERB_GATEWAY: &str = "gateway";
pub const VERB_GATEWAY_WORKTREES: &str = "worktrees";
pub const VERB_WORKTREES_ENSURE: &str = "ensure";
pub const VERB_WORKTREES_REMOVE: &str = "remove";
pub const VERB_TICKET: &str = "ticket";
pub const VERB_TICKET_LABEL: &str = "label";
pub const VERB_TICKET_COMMENT: &str = "comment";
pub const VERB_TICKET_CLOSE: &str = "close";
pub const VERB_GH: &str = "gh";
pub const VERB_GH_PR_READY: &str = "pr-ready";
pub const VERB_GH_PR_MERGE: &str = "pr-merge";

pub const FLAG_BODY_FILE: &str = "--body-file";
pub const FLAG_EXPECTED_HEAD_SHA: &str = "--expected-head-sha";
/// `aidp gh pr-merge` reports a conflict with the base as an output instead of failing.
pub const FLAG_REPORT_CONFLICT: &str = "--report-conflict";
pub const FLAG_BRANCH: &str = "--branch";
/// `aidp flow unity-refresh` flag stretching the compile wait and the heartbeat tolerance.
pub const FLAG_WAIT_MULTIPLIER: &str = "--wait-multiplier";
pub const FLAG_DRAFT: &str = "--draft";
pub const FLAG_COST_FOOTER: &str = "--cost-footer";
pub const FLAG_REPORT_FILE: &str = "--report-file";
pub const FLAG_SUMMARY_PREFIX: &str = "--summary=";
pub const FLAG_ALLOW_EXTRA: &str = "--allow-extra";

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
pub const WITH_REPORT: &str = "report";
pub const WITH_SUMMARY: &str = "summary";
pub const WITH_RUN_DIR: &str = "run_dir";
/// `bug.create` key: one bug past the cap.
pub const WITH_ALLOW_EXTRA: &str = "allow_extra";
/// Run folder of the stage that wrote `with.body`, when it is not the one of `with.run_dir`.
pub const WITH_BODY_DIR: &str = "body_dir";

pub const OUTPUT_SKIPPED: &str = "skipped";
pub const OUTPUT_REASON: &str = "reason";
pub const OUTPUT_WAITING_EXTERNAL: &str = "waiting_external";

pub const GIT_PUSH_TIMEOUT_SECONDS: u64 = 15 * 60;
/// Above the verb's own 600 s wait for the compile result, so the verb reports `blocked` itself.
pub const UNITY_REFRESH_TIMEOUT_SECONDS: u64 = 11 * 60;

pub const LOCAL_KEY_SEPARATOR: &str = "-";

/// `gh.merge` output: the pull request is merged.
pub const OUTPUT_MERGED: &str = "merged";
/// `gh.merge` output: the pull request conflicts with its base and was not merged.
pub const OUTPUT_CONFLICT: &str = "conflict";
