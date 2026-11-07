use aidp_core::exec_plan::StageSpec;
use std::path::PathBuf;

/// One session to run for a stage, a plan task or a shard.
pub struct SessionJob {
    pub stage: StageSpec,
    /// Row name and origin label: the stage name, or `<stage>#<id>`.
    pub label: String,
    pub prompt: String,
    /// Directory the agent writes the reply file into.
    pub reply_dir: PathBuf,
    pub declared_outputs: Vec<String>,
    /// Check and restore writes outside the stage scope once the session ended.
    pub guard_scope: bool,
    /// Directory the session works in, instead of the run workspace (an isolated task worktree).
    pub workspace: Option<PathBuf>,
}
