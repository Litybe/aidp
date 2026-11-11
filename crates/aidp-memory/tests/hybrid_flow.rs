use aidp_memory::{
    Embedder, HybridRetriever, IndexOptions, Indexer, MemoryStore, ProjectRef, Result, Retriever,
    SearchQuery, Source,
};
use std::path::Path;

const PROJECT: &str = "mini";
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mini");
const MODEL: &str = "fake-model";
const OTHER_MODEL: &str = "other-fake-model";
const DIM: usize = 64;
const SYNONYMS: &[(&str, &str)] = &[("bondir", "saut"), ("sauter", "saut")];
const HASH_SEED: usize = 17;
const HASH_FACTOR: usize = 31;

struct HashEmbedder {
    id: &'static str,
}

impl Embedder for HashEmbedder {
    fn model_id(&self) -> &str {
        self.id
    }

    fn dim(&self) -> usize {
        DIM
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0_f32; DIM];
                for word in text
                    .to_lowercase()
                    .split(|character: char| !character.is_alphanumeric())
                    .filter(|word| !word.is_empty())
                {
                    let canonical = SYNONYMS
                        .iter()
                        .find(|(alias, _)| *alias == word)
                        .map_or(word, |(_, target)| *target);
                    let bucket = canonical.bytes().fold(HASH_SEED, |acc, byte| {
                        acc.wrapping_mul(HASH_FACTOR)
                            .wrapping_add(usize::from(byte))
                    }) % DIM;
                    vector[bucket] += 1.0;
                }
                vector
            })
            .collect())
    }
}

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

fn project(workspace: &Path) -> ProjectRef {
    copy_tree(Path::new(FIXTURE), workspace);
    ProjectRef {
        key: PROJECT.into(),
        root: workspace.to_path_buf(),
    }
}

fn embedded_chunks(store: &MemoryStore, model: &str) -> usize {
    store
        .read(|connection| {
            Ok(connection.query_row(
                "SELECT COUNT(*) FROM chunk_vectors WHERE model = ?1",
                [model],
                |row| row.get(0),
            )?)
        })
        .unwrap()
}

#[test]
fn hybrid_search_finds_by_meaning_what_bm25_misses() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let project = project(workspace.path());
    let store = MemoryStore::open_in(gateway.path()).unwrap();
    let embedder = HashEmbedder { id: MODEL };
    let report = Indexer::new(&store)
        .with_embedder(&embedder)
        .index(&project, &IndexOptions::default())
        .unwrap();
    assert_eq!(report.embedded, report.chunks);
    let query = SearchQuery::new(PROJECT, "bondir");
    assert!(aidp_memory::Bm25Retriever::new(&store, None)
        .search(&query)
        .unwrap()
        .is_empty());
    let hybrid = HybridRetriever::new(&store, None, Box::new(HashEmbedder { id: MODEL }));
    let hits = hybrid.search(&query).unwrap();
    assert!(hits.iter().any(|hit| hit.source == Source::Bible));
}

#[test]
fn hybrid_search_keeps_the_lexical_answer_on_top_for_a_literal_query() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let project = project(workspace.path());
    let store = MemoryStore::open_in(gateway.path()).unwrap();
    let embedder = HashEmbedder { id: MODEL };
    Indexer::new(&store)
        .with_embedder(&embedder)
        .index(&project, &IndexOptions::default())
        .unwrap();
    let hybrid = HybridRetriever::new(
        &store,
        Some(project.root.clone()),
        Box::new(HashEmbedder { id: MODEL }),
    );
    let hits = hybrid
        .search(&SearchQuery::new(PROJECT, "gardien de la forêt"))
        .unwrap();
    assert_eq!(hits[0].path, "Docs/GameDesign/boss.md");
}

#[test]
fn changing_the_model_re_embeds_on_the_next_index() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let project = project(workspace.path());
    let store = MemoryStore::open_in(gateway.path()).unwrap();
    let first = HashEmbedder { id: MODEL };
    let second = HashEmbedder { id: OTHER_MODEL };
    let report = Indexer::new(&store)
        .with_embedder(&first)
        .index(&project, &IndexOptions::default())
        .unwrap();
    assert_eq!(embedded_chunks(&store, MODEL), report.chunks);
    let again = Indexer::new(&store)
        .with_embedder(&second)
        .index(&project, &IndexOptions::default())
        .unwrap();
    assert_eq!(again.embedded, report.chunks);
    assert_eq!(embedded_chunks(&store, MODEL), 0);
    assert_eq!(embedded_chunks(&store, OTHER_MODEL), report.chunks);
}

#[test]
fn vectors_survive_a_reopened_database() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let project = project(workspace.path());
    let embedder = HashEmbedder { id: MODEL };
    {
        let store = MemoryStore::open_in(gateway.path()).unwrap();
        Indexer::new(&store)
            .with_embedder(&embedder)
            .index(&project, &IndexOptions::default())
            .unwrap();
    }
    let store = MemoryStore::open_in(gateway.path()).unwrap();
    let hybrid = HybridRetriever::new(&store, None, Box::new(HashEmbedder { id: MODEL }));
    let hits = hybrid.search(&SearchQuery::new(PROJECT, "sauter")).unwrap();
    assert!(!hits.is_empty());
}
