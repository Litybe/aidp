use serde::{Deserialize, Serialize};

use crate::models::constants::mock::DEFAULT_FAILURE_STATUS;

/// Forces a status on every request whose URL contains `path_contains`.
#[derive(Clone, Serialize, Deserialize)]
pub struct InjectedFailure {
    #[serde(default)]
    pub path_contains: String,
    #[serde(default = "default_failure_status")]
    pub status: u16,
    #[serde(default)]
    pub remaining: Option<u64>,
}

fn default_failure_status() -> u16 {
    DEFAULT_FAILURE_STATUS
}
