use crate::models::enums::step::Step;

/// The parameters of a `Step::Loop`: `source` is the stage and signal its condition reads.
pub(crate) struct LoopRequest<'a> {
    pub(crate) container: &'a str,
    pub(crate) source: Option<(&'a str, &'a str)>,
    pub(crate) body_producer: Option<&'a str>,
    pub(crate) max: u32,
    pub(crate) continue_when_exhausted: bool,
    pub(crate) body: &'a [Step],
}
