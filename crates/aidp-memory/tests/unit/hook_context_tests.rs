use super::*;
use crate::models::new_memory::NewMemory;
use crate::store::MemoryStore;

const PROJECT: &str = "pixelhood";

fn active(manager: &MemoryManager<'_>, project: &str, kind: MemoryKind, title: &str, text: &str) {
    let mut new = NewMemory::proposal(project, kind, title, text);
    new.status = MemoryStatus::Active;
    manager.add(new).unwrap();
}

#[test]
fn the_digest_starts_with_preferences_and_includes_global() {
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    active(
        &manager,
        PROJECT,
        MemoryKind::Fact,
        "Un fait",
        "Texte long du fait.",
    );
    active(
        &manager,
        GLOBAL_PROJECT,
        MemoryKind::Preference,
        "Francais",
        "Repondre en francais.",
    );
    let digest = manager.session_digest(PROJECT).unwrap();
    let preference = digest.find("Francais").unwrap();
    let fact = digest.find("Un fait").unwrap();
    assert!(preference < fact);
    assert!(!digest.contains("Texte long"));
}

#[test]
fn an_empty_project_has_no_digest() {
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    assert_eq!(manager.session_digest(PROJECT).unwrap(), "");
}

#[test]
fn a_prompt_about_a_memory_recalls_it_and_marks_it_used() {
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    active(
        &manager,
        PROJECT,
        MemoryKind::Gotcha,
        "Meta des sprites",
        "Le filterMode doit rester Point a chaque import.",
    );
    let block = manager
        .prompt_recall(PROJECT, "pourquoi le filterMode change sur ce sprite ?")
        .unwrap();
    assert!(block.contains("M-1 (gotcha) Meta des sprites: Le filterMode"));
    assert_eq!(manager.get(PROJECT, "M-1").unwrap().uses, 1);
}

#[test]
fn a_prompt_sharing_one_word_recalls_nothing() {
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    active(
        &manager,
        PROJECT,
        MemoryKind::Gotcha,
        "Meta des sprites",
        "Filtre Point.",
    );
    for prompt in ["sprite", "dessine un sprite de chat", "oui"] {
        assert_eq!(
            manager.prompt_recall(PROJECT, prompt).unwrap(),
            "",
            "{prompt}"
        );
    }
    assert_eq!(manager.get(PROJECT, "M-1").unwrap().uses, 0);
}

#[test]
fn a_recall_keeps_the_closest_memories() {
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    for (title, text) in [
        (
            "Dash vitesse joueur",
            "Le dash conserve la vitesse du joueur.",
        ),
        (
            "Dash camera joueur",
            "Le dash secoue la camera autour du joueur.",
        ),
        ("Dash audio joueur", "Le dash joue un son pour le joueur."),
        (
            "Dash particules joueur",
            "Le dash emet des particules pres du joueur.",
        ),
    ] {
        active(&manager, PROJECT, MemoryKind::Fact, title, text);
    }
    let block = manager
        .prompt_recall(PROJECT, "dash joueur vitesse")
        .unwrap();
    assert_eq!(block.lines().count(), 1 + PROMPT_RECALL_MAX);
    assert!(block
        .lines()
        .nth(1)
        .unwrap()
        .contains("Dash vitesse joueur"));
}
