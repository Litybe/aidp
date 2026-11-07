use serde::{Deserialize, Serialize};

/// How a dev flow reports its branch: a standard PR, a draft PR or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrMode {
    Standard,
    Draft,
    None,
}
