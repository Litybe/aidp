use std::collections::BTreeSet;

/// What the validator checks `use` and pipeline templates against.
/// `pipelines` is `None` when the template catalog could not be read.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub actions: BTreeSet<String>,
    pub wait_probes: BTreeSet<String>,
    pub pipelines: Option<BTreeSet<String>>,
}
