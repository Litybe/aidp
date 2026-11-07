//! Tool rules a PR fix session is allowed to use; every other tool is refused.

pub const TOOL_READ: &str = "Read";
pub const TOOL_EDIT: &str = "Edit";
pub const TOOL_WRITE: &str = "Write";
pub const TOOL_GLOB: &str = "Glob";
pub const TOOL_GREP: &str = "Grep";
pub const TOOL_TODO: &str = "TodoWrite";
pub const BASH_GIT_LOCAL: &str = "Bash(git status*)";
pub const BASH_GIT_DIFF: &str = "Bash(git diff*)";
pub const BASH_GIT_LOG: &str = "Bash(git log*)";
pub const BASH_GIT_SHOW: &str = "Bash(git show*)";
pub const BASH_GIT_ADD: &str = "Bash(git add*)";
pub const BASH_GIT_COMMIT: &str = "Bash(git commit*)";
pub const BASH_GIT_FETCH: &str = "Bash(git fetch*)";
pub const BASH_GIT_REBASE: &str = "Bash(git rebase*)";
pub const BASH_GIT_RESTORE: &str = "Bash(git restore*)";
pub const BASH_GIT_RESET: &str = "Bash(git reset*)";
pub const BASH_GIT_REV_PARSE: &str = "Bash(git rev-parse*)";
pub const BASH_AIDP: &str = "Bash(aidp *)";
pub const BASH_CARGO: &str = "Bash(cargo *)";
pub const BASH_DOTNET: &str = "Bash(dotnet *)";
pub const BASH_NPM: &str = "Bash(npm *)";
pub const BASH_NPX: &str = "Bash(npx *)";
pub const BASH_PNPM: &str = "Bash(pnpm *)";
pub const BASH_YARN: &str = "Bash(yarn *)";
pub const BASH_MAKE: &str = "Bash(make *)";
pub const PR_FIX_BASE_ALLOWED_TOOLS: &[&str] = &[
    TOOL_READ,
    TOOL_EDIT,
    TOOL_WRITE,
    TOOL_GLOB,
    TOOL_GREP,
    TOOL_TODO,
    BASH_GIT_LOCAL,
    BASH_GIT_DIFF,
    BASH_GIT_LOG,
    BASH_GIT_SHOW,
    BASH_GIT_ADD,
    BASH_GIT_COMMIT,
    BASH_GIT_FETCH,
    BASH_GIT_REBASE,
    BASH_GIT_RESTORE,
    BASH_GIT_RESET,
    BASH_GIT_REV_PARSE,
    BASH_AIDP,
    BASH_CARGO,
    BASH_DOTNET,
    BASH_NPM,
    BASH_NPX,
    BASH_PNPM,
    BASH_YARN,
    BASH_MAKE,
];
