/// What the start-up catch-up found among the runs that ended while the daemon was down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryCatchUpReport {
    /// Projects whose pipeline folder was scanned.
    pub projects: usize,
    /// Ended runs within the catch-up window.
    pub runs: usize,
    /// Memory proposal files ingested.
    pub memory_files: usize,
    /// Knowledge proposal files ingested.
    pub knowledge_files: usize,
}
