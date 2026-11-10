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
        live: false,
        activity_ms: 0,
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
fn an_incident_takes_three_actions_each_under_its_own_key() {
    let root = TempWorkspace::new("doctor-store-claim");
    let store = IncidentStore::load(root.path());
    let id = store
        .open(candidate(KEY, "implement"), AT)
        .unwrap()
        .candidate
        .id;
    let first = store
        .claim_action(&id, DoctorAction::RetryStep, AT)
        .unwrap();
    assert_eq!(first, action_key(&id, 1));
    store.finish_action(&id, &first, true, "retried", AT);
    let reloaded = IncidentStore::load(root.path());
    let second = reloaded
        .claim_action(&id, DoctorAction::RequeueDev, AT)
        .unwrap();
    assert_eq!(second, action_key(&id, 2));
    assert_ne!(first, second);
    assert!(reloaded
        .claim_action(&id, DoctorAction::CreateBugTicket, AT)
        .is_some());
    assert!(
        reloaded
            .claim_action(&id, DoctorAction::RetryStep, AT)
            .is_none(),
        "a fourth action is never claimed"
    );
    let incident = reloaded.get(&id).unwrap();
    assert_eq!(incident.actions.len(), 3);
    assert_eq!(incident.actions[0].detail, "retried");
    assert!(incident.actions[0].ok);
    assert_eq!(
        incident.outcome.unwrap().action,
        DoctorAction::CreateBugTicket
    );
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

#[test]
fn resumptions_are_counted_per_ticket_over_all_its_incidents_for_24_hours() {
    let root = TempWorkspace::new("doctor-store-retries");
    let store = IncidentStore::load(root.path());
    let recent = chrono::DateTime::from_timestamp_millis(NOW_MS - HOUR_MS)
        .unwrap()
        .to_rfc3339();
    let old = chrono::DateTime::from_timestamp_millis(NOW_MS - 30 * HOUR_MS)
        .unwrap()
        .to_rfc3339();
    for (step, at, action) in [
        ("a", &recent, DoctorAction::RetryStep),
        ("b", &recent, DoctorAction::RetryStep),
        ("c", &old, DoctorAction::RetryStep),
        ("d", &recent, DoctorAction::RequeueDev),
    ] {
        let id = store.open(candidate(KEY, step), AT).unwrap().candidate.id;
        let key = store.claim_action(&id, action, at).unwrap();
        store.finish_action(&id, &key, true, "done", at);
    }
    assert_eq!(store.retries(KEY, NOW_MS), 2);
    assert_eq!(store.retries(OTHER_KEY, NOW_MS), 0);
}

fn cap_candidate(key: &str, started_ms: i64) -> DoctorCandidate {
    DoctorCandidate {
        id: format!("{key}/bug_capped_stop@0.{started_ms}"),
        trigger: DoctorTrigger::BugsNotConverging,
        ..candidate(key, "bug_capped_stop")
    }
}

#[test]
fn an_investigation_for_bugs_that_do_not_converge_is_not_in_the_24_hour_cap() {
    let root = TempWorkspace::new("doctor-store-cap-trigger");
    let store = IncidentStore::load(root.path());
    for candidate in [
        candidate(KEY, "implement"),
        cap_candidate(KEY, 1),
        cap_candidate(KEY, 2),
    ] {
        let id = candidate.id.clone();
        store.open(candidate, AT);
        store.update(&id, AT, |row| row.investigated_ms = Some(NOW_MS - HOUR_MS));
    }
    assert_eq!(store.attempts(KEY, NOW_MS), 1);
}

#[test]
fn a_new_cap_of_the_same_ticket_is_a_new_incident() {
    let root = TempWorkspace::new("doctor-store-cap-again");
    let store = IncidentStore::load(root.path());
    assert!(store.open(cap_candidate(KEY, 1), AT).is_some());
    assert!(store.open(cap_candidate(KEY, 1), AT).is_none());
    assert!(store.open(cap_candidate(KEY, 2), AT).is_some());
}

#[test]
fn a_shared_incident_covers_its_pattern_for_24_hours_unless_it_failed() {
    let root = TempWorkspace::new("doctor-store-covering");
    let store = IncidentStore::load(root.path());
    let id = store.open(cap_candidate(KEY, 1), AT).unwrap().candidate.id;
    let patterns = vec!["no_player_in_scene".to_string()];
    let now = crate::doctor::detect::millis_of(AT).unwrap() + HOUR_MS;
    assert!(store.covering(&patterns, now).is_none());
    store.update(&id, AT, |row| {
        row.shared_pattern = Some("no_player_in_scene".to_string());
        row.status = DoctorIncidentStatus::Detected;
    });
    assert!(store.covering(&patterns, now).is_none(), "still detected");
    store.update(&id, AT, |row| row.status = DoctorIncidentStatus::NeedsHuman);
    assert_eq!(store.covering(&patterns, now).unwrap().candidate.id, id);
    assert!(store.covering(&["other".to_string()], now).is_none());
    assert!(store.covering(&patterns, now + 24 * HOUR_MS).is_none());
    store.update(&id, AT, |row| {
        row.status = DoctorIncidentStatus::InvestigationFailed
    });
    assert!(store.covering(&patterns, now).is_none());
}

#[test]
fn the_cap_of_a_ticket_is_re_armed_once_over_24_hours_for_every_ticket_it_touches() {
    let root = TempWorkspace::new("doctor-store-rearmed");
    let store = IncidentStore::load(root.path());
    let id = store.open(cap_candidate(KEY, 1), AT).unwrap().candidate.id;
    assert!(!store.rearmed_recently(KEY, NOW_MS));
    store.update(&id, AT, |row| {
        row.rearmed_ms = Some(NOW_MS - HOUR_MS);
        row.affected = vec![OTHER_KEY.to_string()];
    });
    assert!(store.rearmed_recently(KEY, NOW_MS));
    assert!(store.rearmed_recently(OTHER_KEY, NOW_MS));
    assert!(!store.rearmed_recently("GH-99", NOW_MS));
    assert!(!store.rearmed_recently(KEY, NOW_MS + 24 * HOUR_MS));
}
