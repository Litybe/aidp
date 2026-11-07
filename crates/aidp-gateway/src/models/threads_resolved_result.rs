use aidp_integrations::models::thread_result::ThreadResult;
use serde::Serialize;

/// Result of a thread resolve or unresolve, one entry per thread id.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsResolvedResult {
    pub unresolve: bool,
    pub results: Vec<ThreadResult>,
}
