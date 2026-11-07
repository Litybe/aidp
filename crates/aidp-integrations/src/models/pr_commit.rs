use serde::Serialize;

/// One commit of a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCommit {
    pub sha: String,
    pub message_headline: String,
    pub author: Option<String>,
    pub authored_date: Option<String>,
}
