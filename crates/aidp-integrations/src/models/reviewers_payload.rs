//! Request body requesting pull request reviewers.

use serde::Serialize;

/// Body of `POST .../requested_reviewers`.
#[derive(Debug, Serialize)]
pub struct ReviewersPayload<'a> {
    pub reviewers: Vec<&'a str>,
}
