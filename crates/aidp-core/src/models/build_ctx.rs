use crate::models::port::PlanSource;
use crate::pipeline::Pipeline;
use crate::run::RunState;
use std::collections::BTreeSet;

/// Everything the compilation reads and never changes.
pub(crate) struct BuildCtx<'a> {
    pub(crate) pipe: &'a Pipeline,
    pub(crate) state: &'a RunState,
    pub(crate) plans: &'a dyn PlanSource,
    pub(crate) soft: BTreeSet<String>,
    pub(crate) loop_linked: BTreeSet<String>,
    pub(crate) loop_reviewed: BTreeSet<String>,
    pub(crate) fanout_producers: BTreeSet<String>,
    pub(crate) plan_producers: BTreeSet<String>,
    pub(crate) approved: Vec<String>,
    pub(crate) budget: usize,
}
