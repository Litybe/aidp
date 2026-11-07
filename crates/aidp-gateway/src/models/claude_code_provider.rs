use std::sync::Arc;

use aidp_core::process_runner::ProcessRunner;

/// Runs the `claude` CLI through an injected [`ProcessRunner`].
pub struct ClaudeCodeProvider {
    pub(crate) runner: Arc<dyn ProcessRunner>,
}
