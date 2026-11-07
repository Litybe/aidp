use aidp_integrations::models::enums::lane::Lane;
use aidp_integrations::models::enums::my_review_state::MyReviewState;
use aidp_integrations::models::squad_pr::SquadPr;
use serde::Serialize;

/// A squad pull request with the lanes it belongs to and my review state.
#[derive(Serialize)]
pub struct SquadPrRow {
    #[serde(flatten)]
    pub pr: SquadPr,
    pub lanes: Vec<Lane>,
    pub my_review_state: MyReviewState,
}
