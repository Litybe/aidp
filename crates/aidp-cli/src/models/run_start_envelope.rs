use serde::Serialize;

use super::vault_location::VaultLocation;

/// Envelope printed by `aidp run start`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct RunStartEnvelope<'a> {
    pub auto_approved_gates: &'a [String],
    pub awaiting_composite: bool,
    pub awaiting_plan: bool,
    pub gates: &'a [String],
    pub last_workflow_status: &'a Option<String>,
    pub run_dir: String,
    pub run_id: &'a str,
    pub script_path: String,
    pub vault: Option<VaultLocation<'a>>,
    pub warnings: Vec<String>,
}
