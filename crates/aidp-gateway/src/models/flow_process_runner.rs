use aidp_core::process_runner::ProcessRunner;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Child processes carrying only the owning runtime's gateway connection and storage.
pub struct FlowProcessRunner {
    pub(crate) inner: Arc<dyn ProcessRunner>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) aidp_program: Option<String>,
}
