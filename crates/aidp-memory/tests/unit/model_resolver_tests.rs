use super::*;
use crate::indexer::Indexer;
use crate::models::index_options::IndexOptions;
use crate::test_support::{fixture_project, HashEmbedder, FAKE_DIM};

fn write_model_files(gateway: &Path) {
    let dir = model_dir(gateway);
    std::fs::create_dir_all(&dir).unwrap();
    for file in MODEL_FILES {
        std::fs::write(dir.join(file), b"").unwrap();
    }
}

#[test]
fn model_dir_sits_under_the_gateway_memory_folder() {
    let dir = model_dir(Path::new("gateway"));
    assert!(dir.ends_with(Path::new("memory/models/potion-multilingual-128M")));
}

#[test]
fn absent_model_resolves_to_none_and_falls_back() {
    let gateway = tempfile::tempdir().unwrap();
    assert!(resolve_model(gateway.path()).is_none());
    assert!(load_default_embedder(gateway.path()).is_none());
}

#[test]
fn incomplete_model_folder_is_absent() {
    let gateway = tempfile::tempdir().unwrap();
    let dir = model_dir(gateway.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MODEL_FILES[0]), b"").unwrap();
    assert!(resolve_model(gateway.path()).is_none());
}

#[test]
fn complete_model_folder_resolves_and_an_unreadable_model_falls_back() {
    let gateway = tempfile::tempdir().unwrap();
    write_model_files(gateway.path());
    assert_eq!(
        resolve_model(gateway.path()),
        Some(model_dir(gateway.path()))
    );
    assert!(load_default_embedder(gateway.path()).is_none());
}

#[test]
fn status_without_model_or_vectors() {
    let gateway = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let status = model_status(&store, gateway.path()).unwrap();
    assert!(!status.present);
    assert_eq!(status.dim, None);
    assert_eq!(status.embedded_chunks, 0);
    assert_eq!(status.model, DEFAULT_MODEL_NAME);
}

#[test]
fn status_counts_vectors_of_the_default_model_and_stale_ones() {
    let gateway = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let project = fixture_project(workspace.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let embedder = HashEmbedder::new(DEFAULT_MODEL_NAME);
    let report = Indexer::new(&store)
        .with_embedder(&embedder)
        .index(&project, &IndexOptions::default())
        .unwrap();
    let status = model_status(&store, gateway.path()).unwrap();
    assert_eq!(status.embedded_chunks, report.chunks);
    assert_eq!(status.total_chunks, report.chunks);
    assert_eq!(status.dim, Some(FAKE_DIM));
    store
        .write(|transaction| {
            transaction.execute("UPDATE chunk_vectors SET model = 'old-model'", [])?;
            Ok(())
        })
        .unwrap();
    let stale = model_status(&store, gateway.path()).unwrap();
    assert_eq!(stale.embedded_chunks, 0);
    assert_eq!(stale.stale_chunks, report.chunks);
}

#[test]
fn weights_bytes_is_the_size_of_the_weights_file_or_none() {
    let gateway = tempfile::tempdir().unwrap();
    assert_eq!(model_weights_bytes(gateway.path()), None);
    let dir = model_dir(gateway.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MODEL_FILES[0]), vec![0u8; 1234]).unwrap();
    assert_eq!(model_weights_bytes(gateway.path()), Some(1234));
}

#[test]
fn status_never_loads_the_model_to_learn_its_size() {
    let gateway = tempfile::tempdir().unwrap();
    write_model_files(gateway.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let status = model_status(&store, gateway.path()).unwrap();
    assert!(status.present);
    assert_eq!(status.dim, None);
}
