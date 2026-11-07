use serde::{Deserialize, Serialize};

/// What a provider supports; callers degrade gracefully on false.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub native_skills: bool,
    pub hooks: bool,
    pub permission_host: bool,
    pub structured_output: bool,
    pub resume: bool,
    pub cost_reporting: bool,
}
