use aidp_integrations::pr::PrState;

/// What the fix needs to know about a pull request, read from GitHub at click time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrFixPull {
    pub author: Option<String>,
    pub viewer: Option<String>,
    pub state: PrState,
    pub head_branch: String,
}
