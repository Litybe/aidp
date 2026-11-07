use crate::models::{ExternalDigest, ExternalManualEntry, InboxItem, WaitRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What the executor started on behalf of a run, keyed by effect id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Runtime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_request: Option<crate::models::TicketFlowRequest>,
    #[serde(default)]
    pub sessions: BTreeMap<String, String>,
    #[serde(default)]
    pub waits: BTreeMap<String, WaitRecord>,
    #[serde(default)]
    pub inbox: Vec<InboxItem>,
    /// Manual actions the external caller has not answered yet.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub external_manual: BTreeMap<String, ExternalManualEntry>,
    /// The last results applied for the external caller, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_results: Vec<ExternalDigest>,
}
