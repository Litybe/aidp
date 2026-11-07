use serde::Serialize;

use crate::models::mock_login::Login;
use crate::models::mock_team_slug::TeamSlug;

/// Reply of the request-reviewers call.
#[derive(Serialize)]
pub struct RequestedReviewersResponse {
    pub requested_reviewers: Vec<Login>,
    pub requested_teams: Vec<TeamSlug>,
}
