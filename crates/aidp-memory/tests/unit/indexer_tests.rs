use super::*;
use crate::test_support::{fixture_project, write, PROJECT};

fn run(store: &MemoryStore, project: &ProjectRef, full: bool) -> IndexReport {
    Indexer::new(store)
        .index(
            project,
            &IndexOptions {
                full,
                ..Default::default()
            },
        )
        .unwrap()
}

#[test]
fn first_pass_indexes_sources_and_ignores_library() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let report = run(&store, &project, false);
    assert_eq!(report.indexed, 5);
    assert_eq!(report.updated, 0);
    assert!(report.chunks >= 8);
    let junk: i64 = store
        .read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM chunks WHERE path LIKE '%Library%'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(junk, 0);
}

#[test]
fn second_pass_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let first = run(&store, &project, false);
    let second = run(&store, &project, false);
    assert_eq!((second.indexed, second.updated, second.removed), (0, 0, 0));
    assert_eq!(second.unchanged, first.indexed);
    assert_eq!(second.chunks, first.chunks);
}

#[test]
fn edited_file_is_rechunked_and_deleted_file_loses_its_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    run(&store, &project, false);
    write(
        dir.path(),
        "Docs/GameDesign/boss.md",
        "# Boss\nNouveau texte du gardien, plus long qu'avant.\n",
    );
    std::fs::remove_file(dir.path().join("Assets/Scripts/Health.cs")).unwrap();
    let report = run(&store, &project, false);
    assert_eq!(report.updated, 1);
    assert_eq!(report.removed, 1);
    let health: i64 = store
        .read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM chunks WHERE path LIKE '%Health.cs'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(health, 0);
    let stale_fts: i64 = store
        .read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'points'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(stale_fts, 0);
}

#[test]
fn full_pass_rechunks_everything() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    let first = run(&store, &project, false);
    let again = run(&store, &project, true);
    assert_eq!(again.updated, first.indexed);
    assert_eq!(again.unchanged, 0);
}

#[test]
fn code_chunks_carry_split_identifier_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    let store = MemoryStore::open_in_memory().unwrap();
    run(&store, &project, false);
    let idents: String = store
        .read(|c| {
            Ok(c.query_row(
                "SELECT idents FROM chunks WHERE title = 'PlayerMovement.Jump'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(idents.contains("apply"));
    assert!(idents.contains("damage"));
}

#[test]
fn extra_roots_feed_runs_and_missing_ones_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let project = fixture_project(dir.path());
    write(
        vault.path(),
        "Pipelines/fixture/feat/run1/plan.md",
        "# Plan\nRefaire le saut.\n",
    );
    let store = MemoryStore::open_in_memory().unwrap();
    let report = Indexer::new(&store)
        .index(
            &project,
            &IndexOptions {
                sources: vec![Source::Run, Source::Ticket],
                extra_roots: vec![IndexRoot {
                    source: Source::Run,
                    dir: vault.path().join("Pipelines/fixture"),
                    prefix: "Pipelines/fixture".into(),
                }],
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(report.indexed, 1);
    assert_eq!(report.skipped_sources, ["ticket"]);
    let path: String = store
        .read(|c| {
            Ok(
                c.query_row("SELECT path FROM chunks WHERE source = 'run'", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(path, "Pipelines/fixture/feat/run1/plan.md");
    assert_eq!(store.stats(PROJECT).unwrap().files, 1);
}

#[test]
fn missing_project_directory_is_not_found() {
    let store = MemoryStore::open_in_memory().unwrap();
    let project = ProjectRef {
        key: "x".into(),
        root: "Z:/does/not/exist".into(),
    };
    let error = Indexer::new(&store)
        .index(&project, &IndexOptions::default())
        .unwrap_err();
    assert!(matches!(error, MemoryError::NotFound(_)));
}
