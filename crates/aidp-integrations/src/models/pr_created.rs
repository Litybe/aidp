//! Result of opening a pull request.

use serde::Serialize;

/// URL of the pull request that was just created.
#[derive(Debug, Serialize)]
pub struct PrCreated {
    pub url: String,
}
