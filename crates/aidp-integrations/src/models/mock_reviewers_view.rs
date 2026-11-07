use serde::Serialize;

use crate::models::mock_login::Login;
use crate::models::mock_team_slug::TeamSlug;

/// Reply of the requested-reviewers listing.
#[derive(Serialize)]
pub struct ReviewersView {
    pub users: Vec<Login>,
    pub teams: Vec<TeamSlug>,
}
