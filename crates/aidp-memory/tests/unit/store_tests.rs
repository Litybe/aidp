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
