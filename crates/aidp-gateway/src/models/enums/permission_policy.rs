//! Whether a session may act without asking.

use serde::{Deserialize, Serialize};

/// Permission policy of a session: act freely or ask first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionPolicy {
    #[default]
    Allow,
    Ask,
}
