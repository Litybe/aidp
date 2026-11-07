//! Where a branch publish runs.

use aidp_core::process_runner::ProcessRunner;
use std::path::Path;

/// The process runner, the worktree to publish, and a directory for the request bodies.
pub struct PublishContext<'a> {
    pub runner: &'a dyn ProcessRunner,
    pub workspace: &'a Path,
    pub scratch: &'a Path,
}
