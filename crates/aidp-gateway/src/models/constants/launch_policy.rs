pub const STRICT_MCP_FLAG: &str = "--strict-mcp-config";
pub const INVALID_DENY_RULE: &str =
    "denied shell prefixes and MCP names must not contain permission-pattern syntax";
pub const INVALID_SCOPED_VAULT: &str = "instance AIDP_VAULT must be an absolute path or off";
/// Shell prefixes denied to every headless Claude session, on top of the host's own: the aidp
/// verbs that publish a branch, open, ready or merge a PR, or launch a session.
pub const HEADLESS_DENIED_COMMANDS: &[&str] = &[
    "aidp flow",
    "aidp gateway",
    "aidp gh pr-create",
    "aidp gh pr-ready",
    "aidp gh pr-update-branch",
    "aidp gh pr-merge",
    "aidp gh pr-fix",
];
/// Set in the environment of every headless session: `aidp` refuses there the verbs above, global options or not.
pub const HEADLESS_SESSION_ENV: &str = "AIDP_HEADLESS_SESSION";
pub const HEADLESS_SESSION_VALUE: &str = "1";
pub const HEADLESS_SESSION_REFUSAL: &str =
    "refused in a headless session: only the flow publishes, merges or launches sessions";
/// Appended to the system prompt of every headless Claude session, after any role persona.
pub const HEADLESS_DENIAL_GUIDANCE: &str = "This session runs unattended in dontAsk mode: only the tool calls the operator's allowlist permits run, anything else is denied at once, and nobody can approve it.\n\
A denial applies to that exact call, never to the whole Bash tool, even when the message says \"Permission to use Bash has been denied\": keep using Bash for the commands the allowlist permits.\n\
Run one simple command per call from the workspace root: never chain with &&, ||, ; or |, and never start with cd; use paths relative to the workspace instead. Your shell instructions may add an \"rtk\" prefix that the allowlist does not name: when an rtk-prefixed command is denied, run the same plain command once. Prefer the dedicated tools (Read, Grep, Glob, Edit, Write) for files.\n\
Before reporting that code was not built or tested, run each build and test command alone (for example dotnet build, dotnet test, cargo test, npm test); only a denial of that plain command is blocking.\n\
Never try to obtain a denied capability another way. When the work needs a command the allowlist does not permit, stop that part, report it as blocking and name the command so the operator can allow it; do not report the stage blocked while it can still be done with permitted calls.";
