/// A stage agent of the daemon carries this variable: its session is not the user's.
pub const ENV_PIPELINE_RUN: &str = "AIDP_RUN";
/// Set on the extraction process so that its own session stays silent.
pub const ENV_DISTILLING: &str = "AIDP_MEMORY_DISTILLING";
/// Value of `ENV_DISTILLING`.
pub const ENV_DISTILLING_VALUE: &str = "1";
/// Model of the extraction.
pub const DISTILL_MODEL: &str = "haiku";
/// Longest extraction, in seconds.
pub const DISTILL_TIMEOUT_SECS: u64 = 180;
/// Delay between two looks at the extraction process, in milliseconds.
pub const DISTILL_POLL_MS: u64 = 500;
/// Ledger of the prompt count already extracted per session, under the memory folder.
pub const DISTILL_LEDGER_FILE: &str = "distilled.json";
/// Outcome of each extraction, under the memory folder.
pub const DISTILL_LOG_FILE: &str = "distill.log";
/// Work folders of the extractions, under the memory folder.
pub const DISTILL_SCRATCH_DIR: &str = "distill-tmp";
/// File the model reply is written to.
pub const DISTILL_REPLY_FILE: &str = "reply.txt";
/// Folder that marks a git repository.
pub const GIT_DIR: &str = ".git";
/// Field of a hook reply that names the event.
pub const EVENT_SESSION_START: &str = "SessionStart";
/// Field of a hook reply that names the event.
pub const EVENT_PROMPT_SUBMIT: &str = "UserPromptSubmit";
/// Program that launches the extraction on Windows.
pub const POWERSHELL_PROGRAM: &str = "powershell";
