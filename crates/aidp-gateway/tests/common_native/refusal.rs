/// How the scripted claude refuses a permission mode.
#[derive(Debug, Clone)]
pub struct Refusal {
    /// The mode the init message reports instead of the asked one.
    pub shown: String,
    /// True when only the preferred bypass mode is refused, false when every mode is.
    pub bypass_only: bool,
}
