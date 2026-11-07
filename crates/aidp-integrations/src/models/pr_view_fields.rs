use crate::pr::PrState;
use serde::Deserialize;

/// Subset of `gh pr view --json` fields read by the PR actions.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrViewFields {
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub state: PrState,
    #[serde(default)]
    pub head_ref_oid: Option<String>,
}
