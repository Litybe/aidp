//! The flow configuration of a repository, after defaults and repo overrides.

use crate::models::flow_caps_config::FlowCapsConfig;
use serde::{Deserialize, Serialize};

/// Every setting the composite flow verbs read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlowRepoConfig {
    pub caps: FlowCapsConfig,
}
