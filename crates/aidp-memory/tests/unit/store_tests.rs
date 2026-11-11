use super::*;

#[test]
fn migrations_run_once_and_set_version() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in(dir.path()).unwrap();
    assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len());
    drop(store);
    let again = MemoryStore::open_in(dir.path()).unwrap();
    assert_eq!(again.schema_version().unwrap(), MIGRATIONS.len());
    assert!(db_path(dir.path()).is_file());
}

#[test]
fn database_uses_wal() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in(dir.path()).unwrap();
    let mode: String = store
        .read(|c| Ok(c.pragma_query_value(None, "journal_mode", |r| r.get(0))?))
        .unwrap();
    assert_eq!(mode, "wal");
}

const PURGED_KEY: &str = ".worktree";
const KEPT_KEY: &str = "pixelhood";

fn add_indexed_file(store: &MemoryStore, project: &str) {
    store
        .write(|tx| {
            tx.execute(
                "INSERT INTO chunks (id, project, source, path, line_start, line_end, title, text, hash, mtime) \
                 VALUES (?1, ?2, 'doc', 'a.md', 1, 1, 'Foret', 'La foret enchantee', 'h', 0)",
                params![format!("chunk-{project}"), project],
            )?;
            tx.execute(
                "INSERT INTO files (project, source, path, size, mtime, hash) VALUES (?1, 'doc', 'a.md', 1, 0, 'h')",
                params![project],
            )?;
            Ok(())
        })
        .unwrap();
}

fn fts_hits(store: &MemoryStore) -> i64 {
    store
        .read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'foret'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap()
}

#[test]
fn purge_removes_one_key_and_leaves_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in(dir.path()).unwrap();
    add_indexed_file(&store, PURGED_KEY);
    add_indexed_file(&store, KEPT_KEY);
    let report = store.purge_index(PURGED_KEY).unwrap();
    assert_eq!(
        report,
        PurgeReport {
            chunks: 1,
            files: 1,
            compacted: true
        }
    );
    assert_eq!(store.stats(KEPT_KEY).unwrap().chunks, 1);
    assert_eq!(store.stats(PURGED_KEY).unwrap().chunks, 0);
    assert_eq!(fts_hits(&store), 1);
}

#[test]
fn purge_of_an_unknown_key_is_not_found() {
    let store = MemoryStore::open_in_memory().unwrap();
    let error = store.purge_index(PURGED_KEY).unwrap_err();
    assert!(matches!(error, MemoryError::NotFound(_)));
}

#[test]
fn purge_refuses_a_key_holding_memories() {
    let store = MemoryStore::open_in_memory().unwrap();
    add_indexed_file(&store, KEPT_KEY);
    crate::manager::MemoryManager::new(&store)
        .add(crate::NewMemory::proposal(
            KEPT_KEY.to_string(),
            crate::MemoryKind::Fact,
            "Titre".to_string(),
            "Texte".to_string(),
        ))
        .unwrap();
    let error = store.purge_index(KEPT_KEY).unwrap_err();
    assert!(matches!(error, MemoryError::Conflict(_)));
    assert_eq!(fts_hits(&store), 1);
}

#[test]
fn fts_handles_french_accents() {
    let store = MemoryStore::open_in_memory().unwrap();
    store
        .write(|tx| {
            tx.execute(
                "INSERT INTO chunks (id, project, source, path, line_start, line_end, title, text, hash, mtime) \
                 VALUES ('i', 'p', 'doc', 'a.md', 1, 1, 'Forêt', 'La héroïne traverse la forêt enchantée', 'h', 0)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let hits: i64 = store
        .read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'heroine AND foret'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(hits, 1);
}

#[test]
fn chunk_vectors_follow_their_chunk_on_delete() {
    let store = MemoryStore::open_in_memory().unwrap();
    store
        .write(|tx| {
            tx.execute(
                "INSERT INTO chunks (id, project, source, path, line_start, line_end, title, text, hash, mtime) \
                 VALUES ('p:doc:a#0', 'p', 'doc', 'a.md', 1, 1, 't', 'x', 'h', 0)",
                [],
            )?;
            let rowid: i64 = tx.query_row("SELECT rowid FROM chunks", [], |row| row.get(0))?;
            tx.execute(
                "INSERT INTO chunk_vectors (chunk_id, model, dim, vector) VALUES (?1, 'm', 1, x'00000000')",
                [rowid],
            )?;
            tx.execute("DELETE FROM chunks", [])?;
            Ok(())
        })
        .unwrap();
    let left: i64 = store
        .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM chunk_vectors", [], |row| row.get(0))?))
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(MIGRATIONS.len(), 3);
}
