use aidp_integrations::pr::PrState;
use serde::Deserialize;

/// One pull request as `gh ... --json number,url,state,headRefName,isDraft` prints it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhPrView {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    #[serde(default)]
    pub head_ref_name: Option<String>,
    #[serde(default)]
    pub is_draft: bool,
}
