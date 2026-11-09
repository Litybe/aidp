use super::*;
use crate::models::enums::DoctorTrigger;
use aidp_testkit::TempWorkspace;

const KEY: &str = "GH-3";
const OTHER_KEY: &str = "GH-4";
const AT: &str = "2026-11-09T12:00:00Z";
const NOW_MS: i64 = 1_800_000_000_000;
const HOUR_MS: i64 = 3_600_000;

fn candidate(key: &str, step: &str) -> DoctorCandidate {
    DoctorCandidate {
        id: format!("{key}/{step}@0"),
        key: key.to_string(),
        ticket: key.to_string(),
        trigger: DoctorTrigger::FlowFailed,
        step: Some(step.to_string()),
        iteration: Some(0),
        run_id: None,
        project_id: None,
        detail: String::new(),
    }
}

#[test]
fn an_incident_is_opened_once_per_id_and_survives_a_reload() {
    let root = TempWorkspace::new("doctor-store-once");
    let store = IncidentStore::load(root.path());
    assert!(store.open(candidate(KEY, "implement"), AT).is_some());
    assert!(store.open(candidate(KEY, "implement"), AT).is_none());
    assert!(store.open(candidate(KEY, "review"), AT).is_some());
    let reloaded = IncidentStore::load(root.path());
    assert_eq!(reloaded.all().len(), 2);
    assert_eq!(
        reloaded.for_key(KEY)[0].candidate.step.as_deref(),
        Some("review")
    );
}

#[test]
fn an_action_is_claimed_once_per_incident() {
    let root = TempWorkspace::new("doctor-store-claim");
    let store = IncidentStore::load(root.path());
    let id = store
        .open(candidate(KEY, "implement"), AT)
        .unwrap()
        .candidate
        .id;
    assert!(store.claim_action(&id, DoctorAction::RetryStep, AT));
    assert!(!store.claim_action(&id, DoctorAction::RetryStep, AT));
    let reloaded = IncidentStore::load(root.path());
    assert!(!reloaded.claim_action(&id, DoctorAction::RequeueDev, AT));
    let outcome = reloaded.get(&id).unwrap().outcome.unwrap();
    assert_eq!(outcome.idempotency_key, action_key(&id));
    assert_eq!(outcome.action, DoctorAction::RetryStep);
}

#[test]
fn attempts_count_the_last_24_hours_of_one_ticket() {
    let root = TempWorkspace::new("doctor-store-attempts");
    let store = IncidentStore::load(root.path());
    for (step, age) in [("a", HOUR_MS), ("b", 2 * HOUR_MS), ("c", 25 * HOUR_MS)] {
        let id = store.open(candidate(KEY, step), AT).unwrap().candidate.id;
        store.update(&id, AT, |row| row.investigated_ms = Some(NOW_MS - age));
    }
    let other = store
        .open(candidate(OTHER_KEY, "a"), AT)
        .unwrap()
        .candidate
        .id;
    store.update(&other, AT, |row| row.investigated_ms = Some(NOW_MS));
    assert_eq!(store.attempts(KEY, NOW_MS), 2);
    assert_eq!(store.attempts(OTHER_KEY, NOW_MS), 1);
}

#[test]
fn one_investigation_at_a_time_is_visible() {
    let root = TempWorkspace::new("doctor-store-investigating");
    let store = IncidentStore::load(root.path());
    let id = store.open(candidate(KEY, "a"), AT).unwrap().candidate.id;
    store.open(candidate(KEY, "b"), AT);
    assert!(store.investigating().is_none());
    store.update(&id, AT, |row| {
        row.status = DoctorIncidentStatus::Investigating
    });
    assert_eq!(store.investigating().unwrap().candidate.id, id);
    assert_eq!(store.detected().len(), 1);
}
