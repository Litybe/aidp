use crate::bus::Bus;
use crate::pr_comment_state::PrCommentState;
use crate::pr_fix::{PrFixGithub, PrFixHost, Sleeper};
use crate::pr_marks::PrMarks;
use crate::projects::Project;
use aidp_core::clock::Clock;
use aidp_core::process_runner::ProcessRunner;
use std::path::Path;
use std::sync::Arc;

/// Every collaborator of one fix, so a test scripts them all.
pub struct PrFixPorts<'a> {
    pub github: Arc<dyn PrFixGithub>,
    /// Whether the run a vault holder names is still running.
    pub run_is_live: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub host: &'a dyn PrFixHost,
    pub runner: Arc<dyn ProcessRunner>,
    pub sleeper: Sleeper,
    pub bus: Arc<Bus>,
    pub clock: &'a dyn Clock,
    pub marks: &'a PrMarks,
    pub comment_state: &'a PrCommentState,
    pub vault_root: &'a Path,
    pub skill_root: &'a Path,
    pub project: &'a Project,
}
