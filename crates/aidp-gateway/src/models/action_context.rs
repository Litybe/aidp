use aidp_core::process_runner::ProcessRunner;
use std::path::Path;

/// What an action sees of the run it belongs to.
pub struct ActionContext<'a> {
    pub key: &'a str,
    pub workspace: Option<&'a Path>,
    pub runner: &'a dyn ProcessRunner,
}
