use crate::models::enums::DoctorCategory;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `diagnosis.json`, as the diagnose stage writes it. The daemon validates it before acting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DoctorDiagnosis {
    pub category: DoctorCategory,
    pub cause: String,
    /// Paths, timestamps and log lines the cause rests on.
    pub evidence: Vec<Value>,
    /// Between 0 and 1.
    pub confidence: f64,
    /// Name of the proposed action; checked against the allowed list.
    pub action: String,
    pub params: Map<String, Value>,
    pub needs_human: bool,
    /// A patch of the harness, proposed and never applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_patch: Option<String>,
    pub summary: String,
}
