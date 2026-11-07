/// Requested reviewers of one pull request: user logins and team slugs.
#[derive(Default, Clone)]
pub struct ReviewerSet {
    pub users: Vec<String>,
    pub teams: Vec<String>,
}
