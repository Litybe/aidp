use std::path::PathBuf;

/// A run directory with its start time, enough to sort and truncate runs.
pub(crate) struct RunStub {
    pub dir: PathBuf,
    pub started_at: String,
}
