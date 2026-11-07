use serde::Deserialize;

/// Body of the request-reviewers call.
#[derive(Deserialize, Default)]
pub struct RequestReviewersBody {
    #[serde(default)]
    pub reviewers: Vec<String>,
    #[serde(default)]
    pub team_reviewers: Vec<String>,
}
