/// State file recording who holds a pull request branch, inside `<vault>/Pipelines/<repo>/PR-<n>/`.
pub const HOLDER_FILE_NAME: &str = "workspace-holder.json";
/// Suffix of the temporary file renamed over the holder file.
pub const HOLDER_TEMP_SUFFIX: &str = ".tmp";
/// Input and action parameter naming the pull request a run or action works on.
pub const INPUT_PR_NUMBER: &str = "pr_number";
/// Input naming the pull request, as the pull request pipelines receive it.
pub const INPUT_PR: &str = "pr";
/// Input naming the target of a run, a pull request number or URL for the pull request pipelines.
pub const INPUT_TARGET: &str = "target";
/// Path segment preceding the number in a pull request URL.
pub const PULL_URL_SEGMENT: &str = "/pull/";
/// Prefix of a pull request reference written as `#123`.
pub const PR_REFERENCE_PREFIX: &str = "#";
/// Suffix of the name of the thread keeping the holder file of a native run up to date.
pub const HOLDER_THREAD_SUFFIX: &str = "holder";
/// Prefix naming a session as holder of the branch in the busy message of the fix route.
pub const HOLDER_SESSION_PREFIX: &str = "session ";
/// Text naming a workflow stage holding the branch when its name is unknown.
pub const HOLDER_WORKFLOW_STAGE: &str = "workflow stage";
/// Text naming a workflow stage in the busy message of the fix route.
pub const HOLDER_STAGE_PREFIX: &str = "workflow stage ";
/// Text naming the run of a workflow stage in the busy message of the fix route.
pub const HOLDER_RUN_INFIX: &str = " of run ";
/// Text naming the session holding the branch in the busy message of the fix route.
pub const HOLDER_SESSION_INFIX: &str = " of session ";
