use crate::models::enums::pass_end::PassEnd;
use crate::models::enums::step::Step;

/// Everything the compilation accumulates.
#[derive(Default)]
pub(crate) struct BuildOut {
    pub(crate) steps: Vec<Step>,
    pub(crate) warnings: Vec<String>,
    pub(crate) gates: Vec<String>,
    pub(crate) emitted: usize,
    pub(crate) end: Option<PassEnd>,
}
