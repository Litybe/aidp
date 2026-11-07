use aidp_memory::models::memory_filter::MemoryFilter;
use aidp_memory::{
    pack, render_markdown, Bm25Retriever, ContextRequest, IndexOptions, Indexer, MemoryKind,
    MemoryManager, MemoryStatus, MemoryStore, NewMemory, ProjectRef, Retriever, Route, SearchQuery,
    Source,
};
use std::path::Path;

const PROJECT: &str = "mini";
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mini");

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn indexed(gateway: &Path, workspace: &Path) -> (MemoryStore, ProjectRef) {
    copy_tree(Path::new(FIXTURE), workspace);
    let project = ProjectRef {
        key: PROJECT.into(),
        root: workspace.to_path_buf(),
    };
    let store = MemoryStore::open_in(gateway).unwrap();
    let report = Indexer::new(&store)
        .index(&project, &IndexOptions::default())
        .unwrap();
    assert_eq!(report.indexed, 5);
    (store, project)
}

#[test]
fn design_question_in_french_returns_the_bible_extract_with_citation() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let (store, project) = indexed(gateway.path(), workspace.path());
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let hits = retriever
        .search(&SearchQuery::new(
            PROJECT,
            "comment marche le double saut de l'héroïne ?",
        ))
        .unwrap();
    let packed = pack(hits, 400);
    let markdown = render_markdown(&packed);
    assert!(markdown.contains(".claude/BIBLE/03_heroine/capacites.md:L3-L4"));
    assert!(markdown.contains("double saut après avoir quitté le sol"));
    assert!(packed.used_tokens <= 400);
}

#[test]
fn code_question_returns_the_method_and_its_graph_neighbour() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let (store, project) = indexed(gateway.path(), workspace.path());
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let mut query = SearchQuery::new(PROJECT, "PlayerMovement Jump");
    query.sources = vec![Source::Code];
    let hits = retriever.search(&query).unwrap();
    let titles: Vec<&str> = hits.iter().map(|hit| hit.title.as_str()).collect();
    assert!(titles.contains(&"PlayerMovement.Jump"));
    let neighbour = hits
        .iter()
        .find(|hit| hit.title == "Health.ApplyDamage")
        .unwrap();
    assert_eq!(neighbour.via, "graph");
    assert_eq!(neighbour.path, "Assets/Scripts/Health.cs");
}

#[test]
fn memory_context_injects_scoped_memories_and_excerpts() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let (store, project) = indexed(gateway.path(), workspace.path());
    let manager = MemoryManager::new(&store);
    let mut gotcha = NewMemory::proposal(
        PROJECT,
        MemoryKind::Gotcha,
        "Hauteur de saut",
        "Ne jamais coder jumpHeight en dur: lire le ScriptableObject.",
    );
    gotcha.status = MemoryStatus::Active;
    gotcha.scope.route = Some(Route::Code);
    manager.add(gotcha).unwrap();
    let mut art = NewMemory::proposal(
        PROJECT,
        MemoryKind::Convention,
        "Palette",
        "Utiliser la palette du projet.",
    );
    art.status = MemoryStatus::Active;
    art.scope.route = Some(Route::Art);
    manager.add(art).unwrap();
    let mut request = ContextRequest::new(PROJECT);
    request.route = Some(Route::Code);
    request.issue_text =
        Some("Le saut de la héroïne: ajuster jumpHeight dans PlayerMovement".into());
    request.budget_tokens = 1200;
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let block = manager.context(&request, &retriever).unwrap();
    assert_eq!(block.memories.len(), 1);
    assert!(block.markdown.contains("Hauteur de saut"));
    assert!(!block.markdown.contains("Palette"));
    assert!(block
        .markdown
        .contains("Assets/Scripts/PlayerMovement.cs:L"));
    assert!(block.used_tokens <= 1200);
    let listed = manager.list(PROJECT, &MemoryFilter::default()).unwrap();
    assert_eq!(listed.iter().find(|m| m.id == "M-1").unwrap().uses, 1);
}

#[test]
fn active_memories_are_searchable_as_the_memory_source() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let (store, _) = indexed(gateway.path(), workspace.path());
    let manager = MemoryManager::new(&store);
    let mut new = NewMemory::proposal(
        PROJECT,
        MemoryKind::Decision,
        "Caméra amortie",
        "La caméra suit avec un lissage exponentiel.",
    );
    new.status = MemoryStatus::Active;
    manager.add(new).unwrap();
    let mut query = SearchQuery::new(PROJECT, "lissage camera");
    query.sources = vec![Source::Memory];
    let hits = Bm25Retriever::new(&store, None).search(&query).unwrap();
    assert_eq!(hits[0].path, "memory/M-1");
    manager.archive(PROJECT, "M-1").unwrap();
    assert!(Bm25Retriever::new(&store, None)
        .search(&query)
        .unwrap()
        .is_empty());
}
