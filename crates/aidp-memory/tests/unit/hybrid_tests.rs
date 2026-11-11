use super::*;
use crate::indexer::Indexer;
use crate::models::enums::Source;
use crate::models::index_options::IndexOptions;
use crate::test_support::{fixture_project, HashEmbedder, FAKE_MODEL, PROJECT};

fn hit(id: &str, via: &str) -> Hit {
    Hit {
        id: id.to_string(),
        source: Source::Doc,
        path: format!("{id}.md"),
        line_start: 1,
        line_end: 1,
        title: id.to_string(),
        score: 0.0,
        text: id.to_string(),
        via: via.to_string(),
    }
}

fn score_of(hits: &[Hit], id: &str) -> f64 {
    hits.iter().find(|hit| hit.id == id).unwrap().score
}

fn indexed_with(embedder_id: &str) -> (MemoryStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let embedder = HashEmbedder::new(embedder_id);
    Indexer::new(&store)
        .with_embedder(&embedder)
        .index(&project, &IndexOptions::default())
        .unwrap();
    (store, dir)
}

#[test]
fn rrf_scores_each_list_by_reciprocal_rank() {
    let fused = rrf_fuse(
        vec![hit("a", "bm25"), hit("b", "bm25")],
        vec![hit("c", "vector"), hit("a", "vector")],
    );
    let expected_a = 1.0 / 61.0 + 1.0 / 62.0;
    assert!((score_of(&fused, "a") - expected_a).abs() < 1e-12);
    assert!((score_of(&fused, "b") - 1.0 / 62.0).abs() < 1e-12);
    assert!((score_of(&fused, "c") - 1.0 / 61.0).abs() < 1e-12);
}

#[test]
fn rrf_puts_the_hit_found_by_both_lists_first_and_marks_it_hybrid() {
    let fused = rrf_fuse(
        vec![hit("a", "bm25"), hit("b", "bm25")],
        vec![hit("c", "vector"), hit("a", "vector")],
    );
    let order: Vec<&str> = fused.iter().map(|hit| hit.id.as_str()).collect();
    assert_eq!(order, vec!["a", "c", "b"]);
    assert_eq!(fused[0].via, "hybrid");
    assert_eq!(fused[1].via, "vector");
    assert_eq!(fused[2].via, "bm25");
}

#[test]
fn rrf_with_an_empty_side_keeps_the_other_order() {
    let fused = rrf_fuse(vec![hit("a", "bm25"), hit("b", "bm25")], Vec::new());
    let order: Vec<&str> = fused.iter().map(|hit| hit.id.as_str()).collect();
    assert_eq!(order, vec!["a", "b"]);
}

#[test]
fn vectors_find_a_synonym_that_bm25_misses() {
    let (store, _dir) = indexed_with(FAKE_MODEL);
    let query = SearchQuery::new(PROJECT, "bondir");
    let lexical = Bm25Retriever::new(&store, None).search(&query).unwrap();
    assert!(lexical.is_empty());
    let hybrid = HybridRetriever::new(&store, None, Box::new(HashEmbedder::new(FAKE_MODEL)));
    let hits = hybrid.search(&query).unwrap();
    assert!(hits
        .iter()
        .any(|hit| hit.title == "Capacités de l'héroïne > Double saut"));
    assert!(hits.iter().all(|hit| hit.via == "vector"));
}

#[test]
fn hybrid_honours_source_and_path_filters() {
    let (store, _dir) = indexed_with(FAKE_MODEL);
    let hybrid = HybridRetriever::new(&store, None, Box::new(HashEmbedder::new(FAKE_MODEL)));
    let mut query = SearchQuery::new(PROJECT, "saut jump");
    query.sources = vec![Source::Code];
    let hits = hybrid.search(&query).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit.source == Source::Code));
    let mut query = SearchQuery::new(PROJECT, "saut");
    query.path_glob = Some("Docs/**".to_string());
    let hits = hybrid.search(&query).unwrap();
    assert!(hits.iter().all(|hit| hit.path.starts_with("Docs/")));
}

#[test]
fn hybrid_ignores_vectors_of_another_model() {
    let (store, _dir) = indexed_with("old-model");
    let hybrid = HybridRetriever::new(&store, None, Box::new(HashEmbedder::new(FAKE_MODEL)));
    let hits = hybrid.search(&SearchQuery::new(PROJECT, "bondir")).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn hybrid_without_any_vector_still_returns_lexical_hits() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    Indexer::new(&store)
        .index(&project, &IndexOptions::default())
        .unwrap();
    let hybrid = HybridRetriever::new(&store, None, Box::new(HashEmbedder::new(FAKE_MODEL)));
    let hits = hybrid
        .search(&SearchQuery::new(PROJECT, "gardien forêt"))
        .unwrap();
    assert_eq!(hits[0].path, "Docs/GameDesign/boss.md");
}
