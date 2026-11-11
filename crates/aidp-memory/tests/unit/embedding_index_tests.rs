use super::*;
use crate::indexer::Indexer;
use crate::models::index_options::IndexOptions;
use crate::store::MemoryStore;
use crate::test_support::{fixture_project, HashEmbedder, FAKE_DIM, FAKE_MODEL};

fn vector_models(store: &MemoryStore) -> Vec<(String, usize)> {
    store
        .read(|connection| {
            let mut statement =
                connection.prepare("SELECT model, dim FROM chunk_vectors ORDER BY chunk_id")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .unwrap()
}

fn chunk_count(store: &MemoryStore) -> usize {
    store
        .read(|connection| {
            Ok(connection.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?)
        })
        .unwrap()
}

#[test]
fn indexing_with_an_embedder_stores_one_vector_per_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let embedder = HashEmbedder::new(FAKE_MODEL);
    let report = Indexer::new(&store)
        .with_embedder(&embedder)
        .index(&project, &IndexOptions::default())
        .unwrap();
    let vectors = vector_models(&store);
    assert_eq!(report.embedded, report.chunks);
    assert_eq!(vectors.len(), chunk_count(&store));
    assert!(vectors
        .iter()
        .all(|(model, dim)| model == FAKE_MODEL && *dim == FAKE_DIM));
}

#[test]
fn unchanged_reindex_embeds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let embedder = HashEmbedder::new(FAKE_MODEL);
    let indexer = Indexer::new(&store).with_embedder(&embedder);
    indexer.index(&project, &IndexOptions::default()).unwrap();
    let again = indexer.index(&project, &IndexOptions::default()).unwrap();
    assert_eq!(again.embedded, 0);
}

#[test]
fn changed_model_re_embeds_every_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let first = HashEmbedder::new("model-a");
    let second = HashEmbedder::new("model-b");
    Indexer::new(&store)
        .with_embedder(&first)
        .index(&project, &IndexOptions::default())
        .unwrap();
    let report = Indexer::new(&store)
        .with_embedder(&second)
        .index(&project, &IndexOptions::default())
        .unwrap();
    let vectors = vector_models(&store);
    assert_eq!(report.embedded, chunk_count(&store));
    assert!(vectors.iter().all(|(model, _)| model == "model-b"));
}

#[test]
fn indexing_without_an_embedder_stores_no_vector() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let report = Indexer::new(&store)
        .index(&project, &IndexOptions::default())
        .unwrap();
    assert_eq!(report.embedded, 0);
    assert!(vector_models(&store).is_empty());
}

#[test]
fn deleting_a_chunk_deletes_its_vector() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let embedder = HashEmbedder::new(FAKE_MODEL);
    Indexer::new(&store)
        .with_embedder(&embedder)
        .index(&project, &IndexOptions::default())
        .unwrap();
    store
        .write(|transaction| {
            transaction.execute("DELETE FROM chunks", [])?;
            Ok(())
        })
        .unwrap();
    assert!(vector_models(&store).is_empty());
}

#[test]
fn more_chunks_than_one_batch_are_all_embedded() {
    let store = MemoryStore::open_in_memory().unwrap();
    let total = EMBED_BATCH * 2 + 5;
    store
        .write(|transaction| {
            for index in 0..total {
                transaction.execute(
                    "INSERT INTO chunks (id, project, source, path, line_start, line_end, title, text, hash, mtime) \
                     VALUES (?1, 'p', 'doc', 'a.md', 1, 1, 'title', ?2, 'h', 0)",
                    params![format!("p:doc:a.md#{index}"), format!("text number {index}")],
                )?;
            }
            Ok(())
        })
        .unwrap();
    let embedder = HashEmbedder::new(FAKE_MODEL);
    let embedded = store
        .write(|transaction| embed_pending(transaction, &embedder, "p", None))
        .unwrap();
    assert_eq!(embedded, total);
    assert_eq!(vector_models(&store).len(), total);
}

#[test]
fn embedding_text_is_title_then_text_cut_to_the_limit() {
    assert_eq!(embedding_text("T", "body"), "T\nbody");
    let long = "x".repeat(EMBED_MAX_CHARS * 2);
    assert_eq!(embedding_text("T", &long).chars().count(), EMBED_MAX_CHARS);
}
