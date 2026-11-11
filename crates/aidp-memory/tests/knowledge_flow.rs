use aidp_memory::models::chunk_meta::ChunkMeta;
use aidp_memory::models::glossary::Glossary;
use aidp_memory::text::match_expression_with;
use aidp_memory::{
    Bm25Retriever, Domain, Embedder, HybridRetriever, IndexOptions, IndexRoot, Indexer,
    MemoryStore, ProjectRef, Result, Retriever, SearchQuery, Source,
};
use std::collections::BTreeSet;
use std::path::Path;

const PROJECT: &str = "mini";
const GLOBAL: &str = "_global";
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/knowledge");
const GLOBAL_PREFIX: &str = "Knowledge/_global";
const MINI_PREFIX: &str = "Knowledge/Mini";
const COYOTE_PATH: &str = "Knowledge/_global/unity/coyote-time.md";
const OUTLINE_PATH: &str = "Knowledge/_global/pixel-art/outline.md";
const UNITY_AGENT: &str = "unity-issue-implementer";
const PIXEL_AGENT: &str = "pixel-artist";
const MODEL: &str = "fake-model";
const FAKE_DIM: usize = 4;

struct ConstantEmbedder;

impl Embedder for ConstantEmbedder {
    fn model_id(&self) -> &str {
        MODEL
    }

    fn dim(&self) -> usize {
        FAKE_DIM
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![1.0; FAKE_DIM]).collect())
    }
}

struct Setup {
    store: MemoryStore,
    project: ProjectRef,
    vault: tempfile::TempDir,
    _workspace: tempfile::TempDir,
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

fn options(vault: &Path, sources: Vec<Source>) -> IndexOptions {
    IndexOptions {
        sources,
        full: false,
        extra_roots: vec![IndexRoot {
            source: Source::Knowledge,
            dir: vault.join("Knowledge/Mini"),
            prefix: MINI_PREFIX.to_string(),
        }],
        global_knowledge: Some(IndexRoot {
            source: Source::Knowledge,
            dir: vault.join("Knowledge/_global"),
            prefix: GLOBAL_PREFIX.to_string(),
        }),
    }
}

fn setup(sources: Vec<Source>) -> Setup {
    let vault = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    copy_tree(Path::new(FIXTURE), vault.path());
    let docs = workspace.path().join("Docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(
        docs.join("tips.md"),
        "# Astuces\n\nLe coyote se retrouve dans la doc.\n",
    )
    .unwrap();
    let project = ProjectRef {
        key: PROJECT.into(),
        root: workspace.path().to_path_buf(),
    };
    let store = MemoryStore::open_in_memory().unwrap();
    Indexer::new(&store)
        .index(&project, &options(vault.path(), sources))
        .unwrap();
    Setup {
        store,
        project,
        vault,
        _workspace: workspace,
    }
}

fn knowledge_setup() -> Setup {
    setup(vec![Source::Knowledge])
}

fn indexed_paths(store: &MemoryStore, project: &str) -> BTreeSet<String> {
    store
        .read(|connection| {
            let mut statement =
                connection.prepare("SELECT DISTINCT path FROM chunks WHERE project = ?1")?;
            let rows = statement.query_map([project], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<_, _>>()?)
        })
        .unwrap()
}

fn search(setup: &Setup, text: &str) -> Vec<aidp_memory::Hit> {
    Bm25Retriever::new(&setup.store, None)
        .search(&SearchQuery::new(PROJECT, text))
        .unwrap()
}

fn paths(hits: &[aidp_memory::Hit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.path.as_str()).collect()
}

#[test]
fn global_notes_are_indexed_under_the_global_project_and_the_rest_is_left_out() {
    let setup = knowledge_setup();
    let global = indexed_paths(&setup.store, GLOBAL);
    let expected: BTreeSet<String> = [
        COYOTE_PATH,
        OUTLINE_PATH,
        "Knowledge/_global/tooling/a-note.md",
        "Knowledge/_global/tooling/b-note.md",
    ]
    .iter()
    .map(|path| path.to_string())
    .collect();
    assert_eq!(global, expected);
    let mini = indexed_paths(&setup.store, PROJECT);
    let expected_mini: BTreeSet<String> = [
        "Knowledge/Mini/notes/mechanics/dash.md",
        "Knowledge/Mini/notes/lore/legend.md",
    ]
    .iter()
    .map(|path| path.to_string())
    .collect();
    assert_eq!(mini, expected_mini);
}

#[test]
fn proposed_superseded_inbox_eval_and_glossary_files_are_never_found() {
    let setup = knowledge_setup();
    for word in ["zorblax", "quuxfrob", "plughxyz", "xyzzy"] {
        assert!(search(&setup, word).is_empty(), "{word}");
    }
    let everything: BTreeSet<String> = indexed_paths(&setup.store, GLOBAL)
        .union(&indexed_paths(&setup.store, PROJECT))
        .cloned()
        .collect();
    assert!(everything
        .iter()
        .all(|path| !path.contains("glossary") && !path.contains("_inbox")));
}

#[test]
fn chunk_meta_carries_the_note_frontmatter() {
    let setup = knowledge_setup();
    let raw: String = setup
        .store
        .read(|connection| {
            Ok(connection.query_row(
                "SELECT meta FROM chunks WHERE project = ?1 AND path = ?2 LIMIT 1",
                [GLOBAL, COYOTE_PATH],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let meta: ChunkMeta = serde_json::from_str(&raw).unwrap();
    assert_eq!(meta.domain.as_deref(), Some("unity"));
    assert_eq!(meta.agents, [UNITY_AGENT, "executor"]);
    assert_eq!(meta.status.as_deref(), Some("active"));
    assert_eq!(meta.importance, Some(0.9));
    assert_eq!(meta.tags, ["movement", "jump"]);
    assert_eq!(meta.kind, "section");
}

#[test]
fn every_note_chunk_title_starts_with_the_note_title() {
    let setup = knowledge_setup();
    let titles: Vec<String> = setup
        .store
        .read(|connection| {
            let mut statement = connection.prepare("SELECT title FROM chunks WHERE path = ?1")?;
            let rows = statement.query_map([COYOTE_PATH], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<_, _>>()?)
        })
        .unwrap();
    assert!(!titles.is_empty());
    assert!(titles
        .iter()
        .all(|title| title.starts_with("Temps de grace au saut")));
}

#[test]
fn a_project_search_also_finds_the_global_notes() {
    let setup = knowledge_setup();
    let hits = search(&setup, "coyote saut plateforme");
    assert!(paths(&hits).contains(&COYOTE_PATH), "{:?}", paths(&hits));
    let other = Bm25Retriever::new(&setup.store, None)
        .search(&SearchQuery::new("someone-else", "coyote saut plateforme"))
        .unwrap();
    assert!(paths(&other).contains(&COYOTE_PATH));
    assert!(!paths(&other).iter().any(|path| path.contains("Mini")));
}

#[test]
fn only_knowledge_chunks_cross_projects() {
    let setup = setup(vec![Source::Knowledge, Source::Doc]);
    let other = Bm25Retriever::new(&setup.store, None)
        .search(&SearchQuery::new("someone-else", "coyote"))
        .unwrap();
    assert!(other.iter().all(|hit| hit.source == Source::Knowledge));
}

#[test]
fn the_domain_filter_keeps_the_listed_domains_only() {
    let setup = knowledge_setup();
    let mut query = SearchQuery::new(PROJECT, "coyote");
    query.domains = vec![Domain::Unity];
    let hits = Bm25Retriever::new(&setup.store, None)
        .search(&query)
        .unwrap();
    assert_eq!(paths(&hits), [COYOTE_PATH]);
    query.domains = vec![Domain::PixelArt];
    let hits = Bm25Retriever::new(&setup.store, None)
        .search(&query)
        .unwrap();
    assert_eq!(paths(&hits), [OUTLINE_PATH]);
    query.domains = vec![Domain::Lore];
    assert!(Bm25Retriever::new(&setup.store, None)
        .search(&query)
        .unwrap()
        .is_empty());
}

#[test]
fn the_agent_filter_keeps_notes_meant_for_the_agent_or_for_all() {
    let setup = knowledge_setup();
    let retriever = Bm25Retriever::new(&setup.store, None);
    let mut query = SearchQuery::new(PROJECT, "coyote");
    query.agent_type = Some(UNITY_AGENT.to_string());
    let both = retriever.search(&query).unwrap();
    assert!(paths(&both).contains(&COYOTE_PATH) && paths(&both).contains(&OUTLINE_PATH));
    query.agent_type = Some(PIXEL_AGENT.to_string());
    assert_eq!(paths(&retriever.search(&query).unwrap()), [OUTLINE_PATH]);
    query.agent_type = Some(format!("plugin:{UNITY_AGENT}"));
    assert!(paths(&retriever.search(&query).unwrap()).contains(&COYOTE_PATH));
}

#[test]
fn filters_leave_other_sources_alone() {
    let setup = setup(vec![Source::Knowledge, Source::Doc]);
    let mut query = SearchQuery::new(PROJECT, "coyote");
    query.domains = vec![Domain::PixelArt];
    query.agent_type = Some(PIXEL_AGENT.to_string());
    let hits = Bm25Retriever::new(&setup.store, None)
        .search(&query)
        .unwrap();
    assert!(hits.iter().any(|hit| hit.source == Source::Doc));
    assert!(hits.iter().any(|hit| hit.path == OUTLINE_PATH));
    assert!(!hits.iter().any(|hit| hit.path == COYOTE_PATH));
}

#[test]
fn importance_raises_the_score_of_equal_notes() {
    let setup = knowledge_setup();
    let hits = search(&setup, "frobnicate widget");
    assert_eq!(
        paths(&hits),
        [
            "Knowledge/_global/tooling/b-note.md",
            "Knowledge/_global/tooling/a-note.md"
        ]
    );
    assert!(hits[0].score > hits[1].score * 1.4, "{:?}", hits);
}

#[test]
fn the_glossary_widens_a_query_to_a_word_no_note_contains() {
    let setup = knowledge_setup();
    assert!(search(&setup, "knockback").is_empty());
    let glossary =
        Glossary::load(&setup.vault.path().join("Knowledge/_glossary.md")).expect("glossary");
    let expression = match_expression_with("knockback", Some(&glossary)).unwrap();
    let found: Vec<String> = setup
        .store
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT c.path FROM chunks_fts JOIN chunks c ON c.rowid = chunks_fts.rowid \
                 WHERE chunks_fts MATCH ?1",
            )?;
            let rows = statement.query_map([expression], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<_, _>>()?)
        })
        .unwrap();
    assert_eq!(found, ["Knowledge/Mini/notes/mechanics/dash.md"]);
}

#[test]
fn global_chunks_are_embedded_and_found_by_the_hybrid_retriever() {
    let vault = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    copy_tree(Path::new(FIXTURE), vault.path());
    let project = ProjectRef {
        key: PROJECT.into(),
        root: workspace.path().to_path_buf(),
    };
    let store = MemoryStore::open_in_memory().unwrap();
    let embedder = ConstantEmbedder;
    let report = Indexer::new(&store)
        .with_embedder(&embedder)
        .index(&project, &options(vault.path(), vec![Source::Knowledge]))
        .unwrap();
    assert!(report.embedded >= 6, "{report:?}");
    let global_vectors: i64 = store
        .read(|connection| {
            Ok(connection.query_row(
                "SELECT COUNT(*) FROM chunk_vectors v JOIN chunks c ON c.rowid = v.chunk_id \
                 WHERE c.project = ?1",
                [GLOBAL],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert!(global_vectors >= 4);
    let hybrid = HybridRetriever::new(&store, None, Box::new(ConstantEmbedder));
    let mut query = SearchQuery::new(PROJECT, "coyote");
    query.domains = vec![Domain::Unity];
    let hits = hybrid.search(&query).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit.path == COYOTE_PATH));
}

#[test]
fn reindexing_drops_a_global_note_that_became_proposed() {
    let setup = knowledge_setup();
    let file = setup
        .vault
        .path()
        .join("Knowledge/_global/unity/coyote-time.md");
    let content = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, content.replace("status: active", "status: proposed")).unwrap();
    Indexer::new(&setup.store)
        .index(
            &setup.project,
            &IndexOptions {
                full: true,
                ..options(setup.vault.path(), vec![Source::Knowledge])
            },
        )
        .unwrap();
    assert!(!indexed_paths(&setup.store, GLOBAL).contains(COYOTE_PATH));
}
