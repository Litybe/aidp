use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// On-disk shape of `pr-viewed.json`: per `owner/repo#number`, file path to the blob sha viewed.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrViewedFile(pub BTreeMap<String, BTreeMap<String, String>>);
