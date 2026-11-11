//! The incidents of the doctor, persisted under `<state dir>/doctor/incidents.json`.

use crate::models::constants::doctor::{
    ACTION_KEY_SUFFIX, ATTEMPT_WINDOW_MS, BUDGET_RAISED_NOTE, DOCTOR_DIR, INCIDENTS_FILE,
    MAX_ACTIONS_PER_INCIDENT,
};
use crate::models::enums::{DoctorAction, DoctorIncidentStatus, DoctorTrigger};
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

/// Idempotency key of attempt `attempt` (from 1) of incident `id`.
pub fn action_key(id: &str, attempt: usize) -> String {
    format!("{id}{ACTION_KEY_SUFFIX}:{attempt}")
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
            actions: Vec::new(),
            next_attempt_ms: None,
            pr_url: None,
            note: None,
            commented: false,
            shared_pattern: None,
            affected: Vec::new(),
            rearmed_ms: None,
            investigations: 0,
            investigation_failures: 0,
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

    /// Investigations ticket `ticket` got in the 24 hours before `now_ms`. The ones opened for bugs
    /// that do not converge are not counted.
    pub fn attempts(&self, ticket: &str, now_ms: i64) -> usize {
        lock(&self.incidents)
            .iter()
            .filter(|incident| incident.candidate.ticket == ticket)
            .filter(|incident| incident.candidate.trigger != DoctorTrigger::BugsNotConverging)
            .filter_map(|incident| {
                incident
                    .investigated_ms
                    .map(|at| (at, incident.investigations.max(1)))
            })
            .filter(|(at, _)| now_ms - at < ATTEMPT_WINDOW_MS)
            .map(|(_, investigations)| investigations as usize)
            .sum()
    }

    /// Ticket ceiling raises `nudge_budget` made for ticket `ticket` in the 24 hours before `now_ms`.
    pub fn budget_raises(&self, ticket: &str, now_ms: i64) -> usize {
        lock(&self.incidents)
            .iter()
            .filter(|incident| incident.candidate.ticket == ticket)
            .flat_map(|incident| incident.actions.iter())
            .filter(|done| {
                done.action == DoctorAction::NudgeBudget
                    && done.ok
                    && done.detail.contains(BUDGET_RAISED_NOTE)
            })
            .filter_map(|done| crate::doctor::detect::millis_of(&done.at))
            .filter(|at| now_ms - at < ATTEMPT_WINDOW_MS)
            .count()
    }

    /// The incident that already covers one of `patterns` for every ticket it touches, opened in
    /// the 24 hours before `now_ms` and not closed on a failure of its own.
    pub fn covering(&self, patterns: &[String], now_ms: i64) -> Option<DoctorIncident> {
        lock(&self.incidents)
            .iter()
            .filter(|incident| {
                !matches!(
                    incident.status,
                    DoctorIncidentStatus::Detected
                        | DoctorIncidentStatus::InvestigationFailed
                        | DoctorIncidentStatus::Capped
                )
            })
            .filter(|incident| {
                incident
                    .shared_pattern
                    .as_ref()
                    .is_some_and(|pattern| patterns.contains(pattern))
            })
            .find(|incident| {
                crate::doctor::detect::millis_of(&incident.detected_at)
                    .is_some_and(|at| now_ms - at < ATTEMPT_WINDOW_MS)
            })
            .cloned()
    }

    /// True when the bug cap of `ticket` was re-armed by an incident in the 24 hours before `now_ms`.
    pub fn rearmed_recently(&self, ticket: &str, now_ms: i64) -> bool {
        lock(&self.incidents).iter().any(|incident| {
            let touches = incident.candidate.ticket == ticket
                || incident.affected.iter().any(|affected| affected == ticket);
            touches
                && incident
                    .rearmed_ms
                    .is_some_and(|at| now_ms - at < ATTEMPT_WINDOW_MS)
        })
    }

    /// Resumptions ticket `ticket` got in the 24 hours before `now_ms`, over all its incidents.
    pub fn retries(&self, ticket: &str, now_ms: i64) -> usize {
        lock(&self.incidents)
            .iter()
            .filter(|incident| incident.candidate.ticket == ticket)
            .flat_map(|incident| incident.actions.iter())
            .filter(|done| done.action == DoctorAction::RetryStep)
            .filter_map(|done| crate::doctor::detect::millis_of(&done.at))
            .filter(|at| now_ms - at < ATTEMPT_WINDOW_MS)
            .count()
    }

    /// Records the next action of incident `id` before it runs, under its own idempotency key
    /// (`<id>:action:<n>`); `None` when the incident used its actions so an attempt never runs twice, even across a restart.
    pub fn claim_action(&self, id: &str, action: DoctorAction, at: &str) -> Option<String> {
        let mut incidents = lock(&self.incidents);
        let incident = incidents
            .iter_mut()
            .find(|incident| incident.candidate.id == id)?;
        if incident.actions.len() >= MAX_ACTIONS_PER_INCIDENT {
            return None;
        }
        let key = action_key(id, incident.actions.len() + 1);
        let outcome = DoctorActionOutcome {
            action,
            idempotency_key: key.clone(),
            ok: false,
            detail: String::new(),
            at: at.to_string(),
        };
        incident.actions.push(outcome.clone());
        incident.outcome = Some(outcome);
        self.persist(&incidents);
        Some(key)
    }

    /// Writes the result of the attempt `key` of incident `id`.
    pub fn finish_action(&self, id: &str, key: &str, ok: bool, detail: &str, at: &str) {
        self.update(id, at, |incident| {
            let mut done = None;
            if let Some(outcome) = incident
                .actions
                .iter_mut()
                .find(|outcome| outcome.idempotency_key == key)
            {
                outcome.ok = ok;
                outcome.detail = detail.to_string();
                outcome.at = at.to_string();
                done = Some(outcome.clone());
            }
            if done.is_some() {
                incident.outcome = done;
            }
        });
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
