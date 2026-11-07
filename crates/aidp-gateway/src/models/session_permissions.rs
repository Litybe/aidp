//! Permissions an autonomous session runs under: a policy and the refused tool patterns.

use serde::{Deserialize, Serialize};

use crate::models::constants::session_profile::DENY_RULES;
use crate::models::enums::permission_policy::PermissionPolicy;

/// The policy and the deny list applied to an autonomous session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPermissions {
    #[serde(default)]
    pub policy: PermissionPolicy,
    #[serde(default)]
    pub deny: Vec<String>,
}

impl SessionPermissions {
    /// `allow` plus the full deny list.
    pub fn autonomous() -> Self {
        Self {
            policy: PermissionPolicy::Allow,
            deny: DENY_RULES.iter().map(|rule| rule.to_string()).collect(),
        }
    }
}
