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
fn context_includes_the_global_memories() {
    let dir = tempfile::tempdir().unwrap();
    let (store, project) = indexed_store(dir.path());
    let manager = MemoryManager::new(&store);
    let mut global = NewMemory::proposal(
        GLOBAL_PROJECT,
        MemoryKind::Preference,
        "Reponses en francais",
        "Toujours repondre en francais.",
    );
    global.status = MemoryStatus::Active;
    manager.add(global).unwrap();
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let block = manager
        .context(&ContextRequest::new(PROJECT), &retriever)
        .unwrap();
    let ids: Vec<&str> = block.memories.iter().map(|m| m.project.as_str()).collect();
    assert_eq!(ids, [GLOBAL_PROJECT]);
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

#[test]
fn the_ticket_route_is_read_from_the_frontmatter() {
    assert_eq!(
        ticket_route("---\nkey: GH-1\nroute: art\n---\nCorps route: map\n"),
        Some(Route::Art)
    );
    assert_eq!(ticket_route("---\nroute: null\n---\n"), None);
    assert_eq!(ticket_route("sans frontmatter"), None);
}

#[test]
fn a_tooling_memory_applies_to_every_route() {
    let dir = tempfile::tempdir().unwrap();
    let (store, project) = indexed_store(dir.path());
    let manager = MemoryManager::new(&store);
    active(
        &manager,
        "Outil MCP",
        "Verifier le MCP avant.",
        Some(Route::Tooling),
        &[],
    );
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let mut request = ContextRequest::new(PROJECT);
    request.route = Some(Route::Art);
    request.record_use = false;
    let block = manager.context(&request, &retriever).unwrap();
    assert_eq!(block.memories.len(), 1);
    assert_eq!(manager.get(PROJECT, "M-1").unwrap().uses, 0);
}

#[test]
fn the_ticket_frontmatter_is_dropped_and_the_summary_leads() {
    let mirror = "---\naidp: ticket\nkey: GH-1\nsummary: 'feat(art): buste de Macaire'\nroute: art\n---\n\n## Description\nUn buste.\n";
    assert_eq!(
        issue_query_text(mirror),
        "feat(art): buste de Macaire\n## Description\nUn buste."
    );
}

#[test]
fn a_memory_sharing_no_word_with_the_query_is_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let (store, project) = indexed_store(dir.path());
    let manager = MemoryManager::new(&store);
    active(
        &manager,
        "Branches",
        "Nommer les branches Feature/PXH-n.",
        None,
        &[],
    );
    active(
        &manager,
        "Meta des sprites",
        "Le meta garde le nom du modele.",
        None,
        &[],
    );
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let mut request = ContextRequest::new(PROJECT);
    request.issue_text = Some("le meta du sprite".into());
    request.record_use = false;
    let block = manager.context(&request, &retriever).unwrap();
    let titles: Vec<&str> = block.memories.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["Meta des sprites"]);
}

#[test]
fn the_best_memory_is_shortened_rather_than_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let (store, project) = indexed_store(dir.path());
    let manager = MemoryManager::new(&store);
    let long_text = format!("Le meta garde le nom du modele. {}", "detail ".repeat(80));
    active(&manager, "Meta des sprites", &long_text, None, &[]);
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let mut request = ContextRequest::new(PROJECT);
    request.issue_text = Some("meta modele".into());
    request.budget_tokens = 100;
    request.record_use = false;
    let block = manager.context(&request, &retriever).unwrap();
    assert_eq!(block.memories.len(), 1);
    assert!(block.memories[0].text.len() < long_text.len());
}

#[test]
fn only_a_plain_issue_key_is_a_path_segment() {
    for ok in ["GH-12", "42", "a.b_c-1"] {
        assert_eq!(safe_issue(ok), Some(ok));
    }
    for bad in ["", "..", "a..b", "../x", "a/b", "a\\b", "C:x", "a b"] {
        assert_eq!(safe_issue(bad), None, "{bad}");
    }
}

#[test]
fn a_route_line_in_a_text_is_found() {
    assert_eq!(
        route_in_text("intro\n  route: art\nsuite"),
        Some(Route::Art)
    );
    assert_eq!(route_in_text("route: 'code'"), Some(Route::Code));
    assert_eq!(route_in_text("route: nothing\nno route"), None);
}
