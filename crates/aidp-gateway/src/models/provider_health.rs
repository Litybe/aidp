use serde::Serialize;

/// One row of the `providers` health report.
#[derive(Debug, Serialize)]
pub struct ProviderHealth {
    pub id: String,
    pub program: String,
    pub found: bool,
    pub version: Option<String>,
}
