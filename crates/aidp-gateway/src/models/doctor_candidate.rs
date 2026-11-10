use crate::models::enums::DoctorTrigger;
use serde::{Deserialize, Serialize};

/// One broken flow or run as detection sees it, before it becomes an incident.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCandidate {
    /// `<key>/<step>@<iteration>` for a flow step, `<key>/run:<run id>` for a run outside a flow.
    pub id: String,
    /// The flow execution key, or the ticket of a run outside a flow.
    pub key: String,
    pub ticket: String,
    pub trigger: DoctorTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Project id of a run outside a flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// What detection saw, in one line.
    pub detail: String,
    /// The failure belongs to a flow that is still going: investigated before the rest.
    #[serde(default)]
    pub live: bool,
    /// Epoch milliseconds of the latest activity behind the failure: newest first.
    #[serde(default)]
    pub activity_ms: i64,
}
