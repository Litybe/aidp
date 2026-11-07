/// Kind of golden-corpus case a template is compiled for.
pub(crate) enum CaseKind {
    Default,
    Plan,
    EnumValue { input: &'static str, value: String },
    ApprovedGate,
    CrossedComposites,
}
