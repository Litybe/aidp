use super::*;
use crate::models::enums::RefType;
use std::sync::{Arc, Mutex};

const PROJECT: &str = "pixelhood";

fn issue(id: &str) -> MemoryRef {
    MemoryRef {
        ref_type: RefType::Issue,
        id: id.to_string(),
    }
}

fn proposal(title: &str, text: &str, issues: &[&str], confidence: f64) -> NewMemory {
    let mut new = NewMemory::proposal(PROJECT, MemoryKind::Lesson, title, text);
    new.refs = issues.iter().map(|id| issue(id)).collect();
    new.confidence = confidence;
    new
}

fn clocked(store: &MemoryStore) -> (MemoryManager<'_>, Arc<Mutex<DateTime<Utc>>>) {
    let now = Arc::new(Mutex::new(
        "2026-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap(),
    ));
    let shared = Arc::clone(&now);
    let manager = MemoryManager::with_clock(store, Box::new(move || *shared.lock().unwrap()));
    (manager, now)
}

#[test]
fn proposal_is_stored_as_proposed_with_evidence_one() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    let outcome = manager
        .add(proposal(
            "Vitesse Rigidbody2D",
            "Régler la vitesse dans FixedUpdate.",
            &["GH-1"],
            0.7,
        ))
        .unwrap();
    assert_eq!(outcome.memory.id, "M-1");
    assert_eq!(outcome.memory.status, MemoryStatus::Proposed);
    assert_eq!(outcome.memory.evidence, 1);
    assert!(!outcome.deduplicated);
}

#[test]
fn similar_proposal_grows_evidence_instead_of_duplicating() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    manager
        .add(proposal(
            "Vitesse du Rigidbody2D en FixedUpdate",
            "Toujours régler la vitesse du Rigidbody2D dans FixedUpdate.",
            &["GH-1"],
            0.7,
        ))
        .unwrap();
    let again = manager
        .add(proposal(
            "Vitesse du Rigidbody2D en FixedUpdate",
            "Toujours régler la vitesse du Rigidbody2D dans FixedUpdate pas Update.",
            &["GH-2"],
            0.75,
        ))
        .unwrap();
    assert!(again.deduplicated);
    assert_eq!(again.memory.id, "M-1");
    assert_eq!(again.memory.evidence, 2);
    assert_eq!(again.memory.refs.len(), 2);
    assert_eq!(
        manager
            .list(PROJECT, &MemoryFilter::default())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unrelated_proposal_is_a_new_memory() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    manager
        .add(proposal(
            "Caméra",
            "Le cadrage suit la héroïne avec un amorti.",
            &[],
            0.7,
        ))
        .unwrap();
    let other = manager
        .add(proposal(
            "Sauvegarde",
            "Écrire le slot avant de quitter la scène.",
            &[],
            0.7,
        ))
        .unwrap();
    assert!(!other.deduplicated);
    assert_eq!(other.memory.id, "M-2");
}

#[test]
fn auto_activation_needs_two_distinct_issues_and_confidence() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    let text = "Ne jamais lire Input dans FixedUpdate sans tampon.";
    manager
        .add(proposal("Input et FixedUpdate", text, &["GH-5"], 0.9))
        .unwrap();
    let same_issue = manager
        .add(proposal("Input et FixedUpdate", text, &["GH-5"], 0.9))
        .unwrap();
    assert_eq!(same_issue.memory.evidence, 2);
    assert_eq!(same_issue.memory.status, MemoryStatus::Proposed);
    let other_issue = manager
        .add(proposal("Input et FixedUpdate", text, &["GH-6"], 0.9))
        .unwrap();
    assert!(other_issue.activated);
    assert_eq!(other_issue.memory.status, MemoryStatus::Active);
}

#[test]
fn low_confidence_never_auto_activates() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    let text = "Préférer un ScriptableObject pour les réglages de saut.";
    manager
        .add(proposal(
            "Réglages en ScriptableObject",
            text,
            &["GH-1"],
            0.5,
        ))
        .unwrap();
    let again = manager
        .add(proposal(
            "Réglages en ScriptableObject",
            text,
            &["GH-2"],
            0.5,
        ))
        .unwrap();
    assert_eq!(again.memory.status, MemoryStatus::Proposed);
}

#[test]
fn approve_archive_and_invalid_transition() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    manager
        .add(proposal("A", "texte alpha unique", &[], 0.7))
        .unwrap();
    assert_eq!(
        manager.approve(PROJECT, "M-1").unwrap().status,
        MemoryStatus::Active
    );
    assert_eq!(
        manager.archive(PROJECT, "M-1").unwrap().status,
        MemoryStatus::Archived
    );
    let error = manager.approve(PROJECT, "M-1").unwrap_err();
    assert!(matches!(error, MemoryError::Conflict(_)));
    assert!(matches!(
        manager.get(PROJECT, "M-9").unwrap_err(),
        MemoryError::NotFound(_)
    ));
}

#[test]
fn supersedes_retires_the_old_memory_on_activation() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    let mut old = proposal(
        "Ancienne décision",
        "On utilise Update pour le mouvement.",
        &[],
        0.7,
    );
    old.status = MemoryStatus::Active;
    manager.add(old).unwrap();
    let mut replacement = proposal(
        "Nouvelle décision",
        "On utilise FixedUpdate pour le mouvement physique.",
        &[],
        0.8,
    );
    replacement.supersedes = Some("M-1".into());
    let created = manager.add(replacement).unwrap().memory;
    assert_eq!(created.status, MemoryStatus::Proposed);
    assert_eq!(
        manager.get(PROJECT, "M-1").unwrap().status,
        MemoryStatus::Active
    );
    manager.approve(PROJECT, &created.id).unwrap();
    assert_eq!(
        manager.get(PROJECT, "M-1").unwrap().status,
        MemoryStatus::Superseded
    );
}

#[test]
fn gc_marks_unused_active_memories_stale_after_sixty_days() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, now) = clocked(&store);
    let mut first = proposal("Vieille", "mémoire ancienne jamais utilisée", &[], 0.7);
    first.status = MemoryStatus::Active;
    manager.add(first).unwrap();
    *now.lock().unwrap() = "2026-02-15T00:00:00Z".parse().unwrap();
    let mut second = proposal(
        "Récente",
        "autre sujet complètement différent ici",
        &[],
        0.7,
    );
    second.status = MemoryStatus::Active;
    manager.add(second).unwrap();
    assert!(manager.gc(PROJECT).unwrap().is_empty());
    *now.lock().unwrap() = "2026-03-10T00:00:00Z".parse().unwrap();
    let stale = manager.gc(PROJECT).unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, "M-1");
    assert_eq!(
        manager.get(PROJECT, "M-1").unwrap().status,
        MemoryStatus::Stale
    );
    assert_eq!(
        manager.get(PROJECT, "M-2").unwrap().status,
        MemoryStatus::Active
    );
}

#[test]
fn merge_adds_evidence_and_supersedes_the_source() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    manager
        .add(proposal("Un", "premier sujet alpha", &["GH-1"], 0.6))
        .unwrap();
    manager
        .add(proposal("Deux", "second thème bêta", &["GH-2"], 0.9))
        .unwrap();
    let merged = manager.merge(PROJECT, "M-1", "M-2").unwrap();
    assert_eq!(merged.evidence, 2);
    assert_eq!(merged.refs.len(), 2);
    assert_eq!(merged.status, MemoryStatus::Active);
    assert_eq!(
        manager.get(PROJECT, "M-1").unwrap().status,
        MemoryStatus::Superseded
    );
    assert!(manager.merge(PROJECT, "M-2", "M-2").is_err());
}

#[test]
fn edit_validates_and_updates() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    manager.add(proposal("Titre", "texte", &[], 0.7)).unwrap();
    let patch = MemoryPatch {
        title: Some("Autre".into()),
        route: Some(crate::models::enums::Route::Code),
        tags: Some(vec!["physics".into()]),
        ..Default::default()
    };
    let edited = manager.edit(PROJECT, "M-1", &patch).unwrap();
    assert_eq!(edited.title, "Autre");
    assert_eq!(edited.scope.tags, ["physics"]);
    let too_long = MemoryPatch {
        text: Some("x".repeat(601)),
        ..Default::default()
    };
    assert!(matches!(
        manager.edit(PROJECT, "M-1", &too_long).unwrap_err(),
        MemoryError::Invalid(_)
    ));
}

#[test]
fn list_filters_by_status_kind_and_query() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    manager
        .add(proposal("Physique", "vitesse du rigidbody", &[], 0.7))
        .unwrap();
    let mut gotcha = NewMemory::proposal(
        PROJECT,
        MemoryKind::Gotcha,
        "Audio",
        "mixer muet au chargement",
    );
    gotcha.status = MemoryStatus::Active;
    manager.add(gotcha).unwrap();
    let filter = |status, kind, query: Option<&str>| MemoryFilter {
        status,
        kind,
        query: query.map(String::from),
    };
    assert_eq!(
        manager
            .list(PROJECT, &filter(Some(MemoryStatus::Active), None, None))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        manager
            .list(PROJECT, &filter(None, Some(MemoryKind::Lesson), None))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        manager
            .list(PROJECT, &filter(None, None, Some("rigid")))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn rejects_oversized_text_and_bad_status() {
    let store = MemoryStore::open_in_memory().unwrap();
    let (manager, _) = clocked(&store);
    assert!(manager
        .add(proposal("T", &"x".repeat(601), &[], 0.7))
        .is_err());
    let mut archived = proposal("T", "ok", &[], 0.7);
    archived.status = MemoryStatus::Archived;
    assert!(manager.add(archived).is_err());
    assert!(manager.add(proposal("T", "ok", &[], 1.5)).is_err());
}
