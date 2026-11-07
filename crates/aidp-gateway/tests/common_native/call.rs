use std::path::PathBuf;

/// What the provider found in the prompt of one launched session.
#[derive(Debug, Clone)]
pub struct Call {
    /// Row name: the stage, or `<stage>#<task>` for a plan task session.
    pub label: String,
    pub role: String,
    /// 1 for the first launch of this label, 2 for the next, and so on.
    pub nth: usize,
    pub stage_dir: PathBuf,
    pub reply_path: PathBuf,
    pub declared_outputs: Vec<String>,
    /// True for a task or shard session, which writes no stage output.
    pub sub_session: bool,
    pub prompt: String,
}
