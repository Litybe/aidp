use serde::{Deserialize, Serialize};

/// What a failing step does to the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    #[default]
    Fail,
    Warn,
}
