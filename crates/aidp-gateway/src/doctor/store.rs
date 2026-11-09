//! The incidents of the doctor, persisted under `<state dir>/doctor/incidents.json`.

use crate::models::constants::doctor::{
    ACTION_KEY_SUFFIX, ATTEMPT_WINDOW_MS, DOCTOR_DIR, INCIDENTS_FILE,
};
use crate::models::enums::{DoctorAction, DoctorIncidentStatus};
use crate::models::{DoctorActionOutcome, DoctorCandidate, DoctorIncident};
use crate::store::{read_json, write_json};
use crate::sync::lock;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Every incident, oldest first; one per id.
pub struct IncidentStore {
    file: PathBuf,
    incidents: Mutex<Vec<DoctorIncident>>,
}

/// Idempotency key of the action of incident `id`.
pub fn action_key(id: &str) -> String {
    format!("{id}{ACTION_KEY_SUFFIX}")
}

impl IncidentStore {
    /// Loads the incidents of `state_dir`; a missing or unreadable file is an empty store.
    pub fn load(state_dir: &Path) -> Self {
        let file = state_dir.join(DOCTOR_DIR).join(INCIDENTS_FILE);
        let incidents = read_json(&file, Vec::new());
        IncidentStore {
            file,
            incidents: Mutex::new(incidents),
        }
    }

    pub fn all(&self) -> Vec<DoctorIncident> {
        lock(&self.incidents).clone()
    }

    pub fn get(&self, id: &str) -> Option<DoctorIncident> {
        lock(&self.incidents)
            .iter()
            .find(|incident| incident.candidate.id == id)
            .cloned()
    }

    /// The incidents of flow or ticket `key`, newest first.
    pub fn for_key(&self, key: &str) -> Vec<DoctorIncident> {
        let mut found: Vec<DoctorIncident> = lock(&self.incidents)
            .iter()
            .filter(|incident| incident.candidate.key == key || incident.candidate.ticket == key)
            .cloned()
            .collect();
        found.reverse();
        found
    }

    /// Files `candidate` as a new incident; `None` when its id is already known.
    pub fn open(&self, candidate: DoctorCandidate, at: &str) -> Option<DoctorIncident> {
        let mut incidents = lock(&self.incidents);
        if incidents
            .iter()
            .any(|incident| incident.candidate.id == candidate.id)
        {
            return None;
        }
        let incident = DoctorIncident {
            candidate,
            status: DoctorIncidentStatus::Detected,
            detected_at: at.to_string(),
            updated_at: at.to_string(),
            investigation_run_id: None,
            investigated_ms: None,
            bundle: None,
            diagnosis: None,
            outcome: None,
            note: None,
            commented: false,
        };
        incidents.push(incident.clone());
        self.persist(&incidents);
        Some(incident)
    }

    /// Applies `change` to incident `id` and saves; the changed incident, or `None` when unknown.
    pub fn update(
        &self,
        id: &str,
        at: &str,
        change: impl FnOnce(&mut DoctorIncident),
    ) -> Option<DoctorIncident> {
        let mut incidents = lock(&self.incidents);
        let incident = incidents
            .iter_mut()
            .find(|incident| incident.candidate.id == id)?;
        change(incident);
        incident.updated_at = at.to_string();
        let changed = incident.clone();
        self.persist(&incidents);
        Some(changed)
    }

    /// The incident whose investigation runs, if any: there is never more than one.
    pub fn investigating(&self) -> Option<DoctorIncident> {
        lock(&self.incidents)
            .iter()
            .find(|incident| incident.status == DoctorIncidentStatus::Investigating)
            .cloned()
    }

    /// Incidents waiting for their investigation, oldest first.
    pub fn detected(&self) -> Vec<DoctorIncident> {
        lock(&self.incidents)
            .iter()
            .filter(|incident| incident.status == DoctorIncidentStatus::Detected)
            .cloned()
            .collect()
    }

    /// Investigations ticket `ticket` got in the 24 hours before `now_ms`.
    pub fn attempts(&self, ticket: &str, now_ms: i64) -> usize {
        lock(&self.incidents)
            .iter()
            .filter(|incident| incident.candidate.ticket == ticket)
            .filter_map(|incident| incident.investigated_ms)
            .filter(|at| now_ms - at < ATTEMPT_WINDOW_MS)
            .count()
    }

    /// Records the action of incident `id` before it runs; false when the incident already
    /// has one, so an action never runs twice, even across a restart.
    pub fn claim_action(&self, id: &str, action: DoctorAction, at: &str) -> bool {
        let mut incidents = lock(&self.incidents);
        let Some(incident) = incidents
            .iter_mut()
            .find(|incident| incident.candidate.id == id)
        else {
            return false;
        };
        if incident.outcome.is_some() {
            return false;
        }
        incident.outcome = Some(DoctorActionOutcome {
            action,
            idempotency_key: action_key(id),
            ok: false,
            detail: String::new(),
            at: at.to_string(),
        });
        self.persist(&incidents);
        true
    }

    fn persist(&self, incidents: &[DoctorIncident]) {
        if let Err(error) = write_json(&self.file, &incidents) {
            tracing::warn!(%error, "doctor incidents not saved");
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_store_tests.rs"]
mod tests;
