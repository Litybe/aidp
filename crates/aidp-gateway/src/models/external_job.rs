use crate::models::agent_job_listing::AgentJobListing;
use crate::models::enums::ExternalJobKind;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// An action or probe the external caller must run, as `GET /api/flows/{key}/external` lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalJob {
    /// The effect id, `<step>:<iteration>:<kind>`.
    pub id: String,
    pub kind: ExternalJobKind,
    pub step: String,
    pub name: String,
    pub attempt: u32,
    pub manual: bool,
    pub writes: bool,
    pub with: BTreeMap<String, Value>,
    pub ticket_key: String,
    pub local: bool,
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_config: Option<Value>,
    pub created_at: String,
    /// Probes only: when the wait began, on the gateway clock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_ms: Option<i64>,
    /// Probes only: the gateway clock at listing time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now_ms: Option<i64>,
    /// Probes only: the memory the last `pending` result stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
    /// Agent jobs only: the session the host is asked to run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentJobListing>,
}
