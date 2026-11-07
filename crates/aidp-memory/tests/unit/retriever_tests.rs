use super::*;
use crate::test_support::{indexed_store, PROJECT};

fn search(store: &MemoryStore, root: Option<PathBuf>, text: &str) -> Vec<Hit> {
    Bm25Retriever::new(store, root)
        .search(&SearchQuery::new(PROJECT, text))
        .unwrap()
}

#[test]
fn french_question_with_accents_finds_the_bible_section() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = indexed_store(dir.path());
    let hits = search(
        &store,
        None,
        "Comment fonctionne le double saut de l'héroïne ?",
    );
    assert_eq!(hits[0].source, Source::Bible);
    assert_eq!(hits[0].title, "Capacités de l'héroïne > Double saut");
    assert!(hits[0]
        .citation()
        .starts_with(".claude/BIBLE/03_heroine/capacites.md:L"));
}

#[test]
fn prefix_and_plural_match() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = indexed_store(dir.path());
    let hits = search(&store, None, "gardiens forets");
    assert_eq!(hits[0].path, "Docs/GameDesign/boss.md");
}

#[test]
fn camel_case_query_hits_split_identifiers() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = indexed_store(dir.path());
    let hits = search(&store, None, "apply damage");
    assert_eq!(hits[0].title, "Health.ApplyDamage");
}

#[test]
fn source_and_path_filters_apply() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = indexed_store(dir.path());
    let retriever = Bm25Retriever::new(&store, None);
    let mut query = SearchQuery::new(PROJECT, "saut jump");
    query.sources = vec![Source::Code];
    let hits = retriever.search(&query).unwrap();
    assert!(hits.iter().all(|h| h.source == Source::Code));
    query.sources = Vec::new();
    query.path_glob = Some("Docs/**".into());
    let hits = retriever.search(&query).unwrap();
    assert!(hits.iter().all(|h| h.path.starts_with("Docs/")));
}

#[test]
fn graph_expansion_adds_called_neighbours_with_damped_score() {
    let dir = tempfile::tempdir().unwrap();
    let (store, project) = indexed_store(dir.path());
    let without = search(&store, None, "jumpHeight Jump");
    assert!(without.iter().all(|h| h.via == "bm25"));
    let with = search(&store, Some(project.root.clone()), "Jump");
    let neighbour = with
        .iter()
        .find(|h| h.title == "Health.ApplyDamage")
        .expect("neighbour via calls edge");
    assert_eq!(neighbour.via, "graph");
    let direct = with
        .iter()
        .find(|h| h.title == "PlayerMovement.Jump")
        .unwrap();
    assert!(neighbour.score < direct.score);
}

#[test]
fn unmatched_query_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = indexed_store(dir.path());
    assert!(search(&store, None, "zzzzqqq").is_empty());
}

#[test]
fn invalid_glob_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = indexed_store(dir.path());
    let mut query = SearchQuery::new(PROJECT, "saut");
    query.path_glob = Some("[".into());
    assert!(Bm25Retriever::new(&store, None).search(&query).is_err());
}
