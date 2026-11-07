use serde::{Deserialize, Serialize};

/// Which part of the inbox could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SquadDegradedKind {
    Team,
    Batch,
    Repo,
}
