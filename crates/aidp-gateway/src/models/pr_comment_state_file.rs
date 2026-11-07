use crate::models::enums::PointState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-PR comment state: ignored points and ids already handed to a `pr-comments` run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrCommentStateFile {
    #[serde(default)]
    pub points: BTreeMap<String, PointState>,
    #[serde(default)]
    pub handed: Vec<String>,
}
