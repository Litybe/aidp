use super::*;
use crate::models::enums::{MemoryKind, Route};
use crate::models::new_memory::NewMemory;
use crate::retriever::Bm25Retriever;
use crate::store::MemoryStore;
use crate::test_support::{indexed_store, PROJECT};

fn active(manager: &MemoryManager, title: &str, text: &str, route: Option<Route>, paths: &[&str]) {
    let mut new = NewMemory::proposal(PROJECT, MemoryKind::Gotcha, title, text);
    new.status = MemoryStatus::Active;
    new.scope.route = route;
    new.scope.paths = paths.iter().map(|p| p.to_string()).collect();
    manager.add(new).unwrap();
}

#[test]
fn extracts_project_paths_from_text() {
    let paths = extract_paths("Voir Assets/Scripts/Player/Jump.cs, et Docs/GameDesign/saut.md.");
    assert_eq!(
        paths,
        ["Assets/Scripts/Player/Jump.cs", "Docs/GameDesign/saut.md"]
    );
}

#[test]
fn context_filters_by_route_and_path_and_marks_use() {
    let dir = tempfile::tempdir().unwrap();
    let (store, project) = indexed_store(dir.path());
    let manager = MemoryManager::new(&store);
    active(
        &manager,
        "Saut: tampon",
        "Garder un tampon de saut de 0.1s.",
        Some(Route::Code),
        &["Assets/Scripts/**"],
    );
    active(
        &manager,
        "Palette",
        "Utiliser la palette du projet.",
        Some(Route::Art),
        &[],
    );
    active(
        &manager,
        "Docs du saut",
        "Mettre à jour le saut dans les docs.",
        None,
        &["Docs/**"],
    );
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let mut request = ContextRequest::new(PROJECT);
    request.route = Some(Route::Code);
    request.issue_text = Some("Corriger le saut dans Assets/Scripts/PlayerMovement.cs".into());
    let block = manager.context(&request, &retriever).unwrap();
    let ids: Vec<&str> = block.memories.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["M-1"]);
    assert!(block.markdown.contains("**M-1**"));
    assert!(block.markdown.contains("## Extraits pertinents"));
    assert!(block
        .excerpts
        .iter()
        .any(|h| h.path == "Assets/Scripts/PlayerMovement.cs"));
    let used = manager.get(PROJECT, "M-1").unwrap();
    assert_eq!(used.uses, 1);
    assert!(used.last_used_at.is_some());
    assert_eq!(manager.get(PROJECT, "M-2").unwrap().uses, 0);
}

#[test]
fn context_respects_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let (store, project) = indexed_store(dir.path());
    let manager = MemoryManager::new(&store);
    for number in 0..10 {
        active(
            &manager,
            &format!("Mémoire {number}"),
            &format!("Leçon numéro {number}: {}", "détail ".repeat(40)),
            None,
            &[],
        );
    }
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let mut request = ContextRequest::new(PROJECT);
    request.budget_tokens = 300;
    request.issue_text = Some("double saut de l'héroïne".into());
    let block = manager.context(&request, &retriever).unwrap();
    assert!(block.used_tokens <= 300);
    assert!(estimate_tokens(&block.markdown) <= 330);
    assert!(block.memories.len() < 10);
}

#[test]
fn empty_context_without_memories_or_issue() {
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let retriever = Bm25Retriever::new(&store, None);
    let block = manager
        .context(&ContextRequest::new(PROJECT), &retriever)
        .unwrap();
    assert!(block.markdown.is_empty());
}
