use crate::models::enums::squad_degraded_kind::SquadDegradedKind;
use serde::{Deserialize, Serialize};

/// A part of the inbox that could not be read, with what to do about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SquadDegraded {
    pub kind: SquadDegradedKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
    pub message: String,
}
