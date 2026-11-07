/// The provider chosen for a stage and its concrete model, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub provider: String,
    pub model: Option<String>,
}
