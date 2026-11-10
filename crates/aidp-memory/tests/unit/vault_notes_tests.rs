use super::*;
use crate::manager::MemoryManager;
use crate::models::enums::{MemoryKind, Route};
use crate::models::new_memory::NewMemory;
use crate::store::MemoryStore;

const PROJECT: &str = "pixelhood";
const TITLE: &str = "Vitesse du Rigidbody en FixedUpdate";
const TEXT: &str = "Passer par PlayerMotor.SetVelocity.";
const EDITED_TEXT: &str = "Passer par le moteur du joueur.";
const NOTE_RELATIVE: &str = "code/lesson/M-1-vitesse-du-rigidbody-en-fixedupdate.md";
const HAND_WRITTEN: &str = "---\nroute: art\n---\n\n# Palette du biome\n\nRampe froide.\n";
const BROKEN: &str = "pas de frontmatter";

fn add_memory(manager: &MemoryManager<'_>) -> String {
    let mut new = NewMemory::proposal(PROJECT, MemoryKind::Lesson, TITLE, TEXT);
    new.scope.route = Some(Route::Code);
    manager.add(new).unwrap().memory.id
}

fn only_memory(manager: &MemoryManager<'_>) -> Memory {
    manager
        .list(PROJECT, &Default::default())
        .unwrap()
        .remove(0)
}

#[test]
fn a_memory_is_exported_once_at_its_route_and_kind() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    add_memory(&manager);
    assert_eq!(manager.sync_vault(PROJECT, dir.path()).unwrap().exported, 1);
    assert!(dir.path().join(NOTE_RELATIVE).is_file());
    let again = manager.sync_vault(PROJECT, dir.path()).unwrap();
    assert_eq!(again, VaultSyncReport::default());
}

#[test]
fn an_edited_note_updates_the_memory() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    add_memory(&manager);
    manager.sync_vault(PROJECT, dir.path()).unwrap();
    let path = dir.path().join(NOTE_RELATIVE);
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, content.replace(TEXT, EDITED_TEXT)).unwrap();
    let report = manager.sync_vault(PROJECT, dir.path()).unwrap();
    assert_eq!((report.imported, report.rejected.len()), (1, 0));
    assert_eq!(only_memory(&manager).text, EDITED_TEXT);
}

#[test]
fn a_status_change_in_a_note_moves_it_to_the_archive() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    add_memory(&manager);
    manager.sync_vault(PROJECT, dir.path()).unwrap();
    let path = dir.path().join(NOTE_RELATIVE);
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        content.replace("status: proposed", "status: archived"),
    )
    .unwrap();
    manager.sync_vault(PROJECT, dir.path()).unwrap();
    assert_eq!(only_memory(&manager).status, MemoryStatus::Archived);
    assert!(!path.exists());
    assert_eq!(
        std::fs::read_dir(dir.path().join(ARCHIVE_DIR))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn a_deleted_note_archives_its_memory_when_other_notes_remain() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    add_memory(&manager);
    manager.sync_vault(PROJECT, dir.path()).unwrap();
    std::fs::remove_file(dir.path().join(NOTE_RELATIVE)).unwrap();
    std::fs::write(dir.path().join("autre.md"), HAND_WRITTEN).unwrap();
    let report = manager.sync_vault(PROJECT, dir.path()).unwrap();
    assert_eq!(report.archived, 1);
    assert_eq!(
        manager.get(PROJECT, "M-1").unwrap().status,
        MemoryStatus::Archived
    );
}

#[test]
fn an_empty_vault_never_archives() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    add_memory(&manager);
    manager.sync_vault(PROJECT, dir.path()).unwrap();
    let other = tempfile::tempdir().unwrap();
    let missing = other.path().join("absent");
    let report = manager.sync_vault(PROJECT, &missing).unwrap();
    assert_eq!((report.archived, report.exported), (0, 1));
    assert_eq!(only_memory(&manager).status, MemoryStatus::Proposed);
}

#[test]
fn a_hand_written_note_becomes_an_active_memory_with_an_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let hand_written = dir.path().join("palette.md");
    std::fs::write(&hand_written, HAND_WRITTEN).unwrap();
    let report = manager.sync_vault(PROJECT, dir.path()).unwrap();
    assert_eq!((report.created, report.rejected.len()), (1, 0));
    let memory = only_memory(&manager);
    assert_eq!(
        (memory.id.as_str(), memory.status, memory.scope.route),
        ("M-1", MemoryStatus::Active, Some(Route::Art))
    );
    assert!(!hand_written.exists());
    assert!(dir
        .path()
        .join("art/fact/M-1-palette-du-biome.md")
        .is_file());
}

#[test]
fn the_database_is_rebuilt_from_the_notes() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    add_memory(&manager);
    manager.sync_vault(PROJECT, dir.path()).unwrap();
    let fresh = MemoryStore::open_in_memory().unwrap();
    let rebuilt = MemoryManager::new(&fresh);
    let report = rebuilt.sync_vault(PROJECT, dir.path()).unwrap();
    assert_eq!(report.created, 1);
    let memory = only_memory(&rebuilt);
    assert_eq!(
        (
            memory.id.as_str(),
            memory.title.as_str(),
            memory.text.as_str()
        ),
        ("M-1", TITLE, TEXT)
    );
    assert_eq!(
        rebuilt.sync_vault(PROJECT, dir.path()).unwrap(),
        VaultSyncReport::default()
    );
}

#[test]
fn a_broken_note_is_reported_kept_and_does_not_archive() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    add_memory(&manager);
    manager.sync_vault(PROJECT, dir.path()).unwrap();
    let path = dir.path().join(NOTE_RELATIVE);
    std::fs::write(&path, BROKEN).unwrap();
    let report = manager.sync_vault(PROJECT, dir.path()).unwrap();
    assert_eq!((report.rejected.len(), report.archived), (1, 0));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), BROKEN);
    assert_eq!(only_memory(&manager).status, MemoryStatus::Proposed);
}
