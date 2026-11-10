/// Folder of the session journals under `<vault>/Knowledge/<repo>`.
pub const JOURNAL_DIR: &str = "_journal";
/// Token budget of the digest injected when a session starts.
pub const SESSION_DIGEST_BUDGET: usize = 500;
/// Most memories injected for one prompt.
pub const PROMPT_RECALL_MAX: usize = 3;
/// Query words a memory must share with a prompt to be injected.
pub const PROMPT_MIN_SHARED_WORDS: usize = 2;
/// Token budget of the memories injected for one prompt.
pub const PROMPT_RECALL_BUDGET: usize = 400;
/// Start of a prompt that a pipeline stage sends to its agent.
pub const PIPELINE_PROMPT_PREFIX: &str = "# Stage `";
/// First character of a harness message inside a user turn.
pub const HARNESS_MESSAGE_PREFIX: char = '<';
/// Most prompts kept in a journal.
pub const JOURNAL_MAX_PROMPTS: usize = 30;
/// Most edited files kept in a journal.
pub const JOURNAL_MAX_FILES: usize = 60;
/// Most commits kept in a journal.
pub const JOURNAL_MAX_COMMITS: usize = 20;
/// Longest prompt kept in a journal, in characters.
pub const JOURNAL_PROMPT_CHARS: usize = 300;
/// Longest final reply kept in a journal, in characters.
pub const JOURNAL_REPLY_CHARS: usize = 800;
/// Longest heading of a journal, in characters.
pub const JOURNAL_HEADING_CHARS: usize = 80;
/// Characters of the session id in a journal file name.
pub const JOURNAL_SESSION_CHARS: usize = 8;
/// Prompts gained since the last extraction before a finished turn triggers another.
pub const DISTILL_EVERY_PROMPTS: usize = 5;
/// Fewest prompts worth an extraction when a session ends or compacts.
pub const DISTILL_MIN_PROMPTS: usize = 2;
/// Tools whose input names an edited file.
pub const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
/// Tools whose input is a shell command.
pub const SHELL_TOOLS: [&str; 2] = ["Bash", "PowerShell"];
/// Commit subject in a shell command: `-m "x"`, a heredoc or a PowerShell here-string.
pub const COMMIT_PATTERN: &str = r#"(?s)git\b[^\n]*\bcommit\b.*?(?:-m\s+["']([^"'\r\n]+)|<<-?\s*'?\w+'?\r?\n([^\r\n]+)|@'\r?\n([^\r\n]+))"#;
