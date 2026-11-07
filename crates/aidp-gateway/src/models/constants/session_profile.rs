//! Constants of the autonomous session profile: environment switch, launch flag and deny rules.

pub const AUTONOMOUS_ENV: &str = "AIDP_AUTONOMOUS";
pub const AUTONOMOUS_ENABLED: &str = "1";
pub const PERMISSIONS_FILE: &str = "permissions.json";
pub const DISALLOWED_TOOLS_FLAG: &str = "--disallowedTools";
pub const ASK_USER_QUESTION_TOOL: &str = "AskUserQuestion";
pub const DENY_FORCE_PUSH: &str = "Bash(git push --force*)";
pub const DENY_FORCE_PUSH_SHORT: &str = "Bash(git push -f*)";
pub const DENY_FORCE_WITH_LEASE: &str = "Bash(git push --force-with-lease*)";
pub const DENY_PUSH_MASTER: &str = "Bash(git push origin master*)";
pub const DENY_PUSH_MAIN: &str = "Bash(git push origin main*)";
pub const DENY_PUSH_DELETE: &str = "Bash(git push --delete*)";
pub const DENY_PUSH_DELETE_REF: &str = "Bash(git push origin :*)";
pub const DENY_PR_MERGE: &str = "Bash(gh pr merge*)";
pub const DENY_PR_CLOSE: &str = "Bash(gh pr close*)";
pub const DENY_RM_RECURSIVE: &str = "Bash(rm -rf*)";
pub const DENY_AIDP_CONFIG_EDIT: &str = "Edit(~/.aidp/**)";
pub const DENY_AIDP_CONFIG_WRITE: &str = "Write(~/.aidp/**)";
pub const DENY_ENV_FILES: &str = "Read(**/.env*)";
pub const DENY_SSH_KEYS: &str = "Read(~/.ssh/**)";
pub const DENY_AWS_CREDENTIALS: &str = "Read(~/.aws/**)";
pub const ALLOWED_TOOLS_FLAG: &str = "--allowedTools";
pub const ADD_DIR_FLAG: &str = "--add-dir";
pub const DENY_PUSH_PLUS_REFSPEC: &str = "Bash(git push origin +*)";
pub const DENY_PUSH_MIRROR: &str = "Bash(git push --mirror*)";
pub const DENY_PUSH_ALL: &str = "Bash(git push --all*)";
pub const DENY_PUSH_TAGS: &str = "Bash(git push --tags*)";
pub const DENY_PUSH_PRUNE: &str = "Bash(git push --prune*)";
pub const PUSH_BRANCH_DENY_RULES: &[&str] = &[
    DENY_PUSH_PLUS_REFSPEC,
    DENY_PUSH_MIRROR,
    DENY_PUSH_ALL,
    DENY_PUSH_TAGS,
    DENY_PUSH_PRUNE,
];
pub const ALLOW_PUSH_TRACKING: &str = "Bash(git push)";
pub const ALLOW_PUSH_ORIGIN_PREFIX: &str = "Bash(git push origin ";
pub const ALLOW_PUSH_HEAD_PREFIX: &str = "Bash(git push origin HEAD:";
pub const RULE_SUFFIX: &str = ")";
pub const BRANCH_DOUBLE_DOT: &str = "..";
pub const BRANCH_FORBIDDEN_LEADING: char = '-';
pub const BRANCH_FORBIDDEN_CHARS: &[char] = &['*', '(', ')', '~', '^', ':', '?', '[', '\\'];
pub const INVALID_PUSH_BRANCH_MESSAGE: &str = "invalid push branch name";
pub const DENY_RULES: &[&str] = &[
    DENY_FORCE_PUSH,
    DENY_FORCE_PUSH_SHORT,
    DENY_FORCE_WITH_LEASE,
    DENY_PUSH_MASTER,
    DENY_PUSH_MAIN,
    DENY_PUSH_DELETE,
    DENY_PUSH_DELETE_REF,
    DENY_PR_MERGE,
    DENY_PR_CLOSE,
    DENY_RM_RECURSIVE,
    DENY_AIDP_CONFIG_EDIT,
    DENY_AIDP_CONFIG_WRITE,
    DENY_ENV_FILES,
    DENY_SSH_KEYS,
    DENY_AWS_CREDENTIALS,
];
