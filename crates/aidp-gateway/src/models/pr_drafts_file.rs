use aidp_integrations::models::review_draft::ReviewDraft;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// On-disk shape of `pr-drafts.json`: drafts keyed by `owner/repo#number`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrDraftsFile(pub BTreeMap<String, Vec<ReviewDraft>>);
