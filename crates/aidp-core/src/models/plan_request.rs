use std::collections::BTreeMap;

/// The plan-driven parameters of a `Step::PlanTasks`.
pub(crate) struct PlanRequest<'a> {
    pub(crate) from: &'a str,
    pub(crate) plan_path: &'a str,
    pub(crate) max: u32,
    pub(crate) kinds: &'a [String],
    pub(crate) merge_hint: Option<&'a str>,
    pub(crate) skills_by_kind: &'a BTreeMap<String, Vec<String>>,
}
