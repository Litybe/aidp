use aidp_core::context_provider::ContextProvider;
use aidp_core::models::enums::KnowledgeRoute;
use aidp_core::models::stage_context_request::StageContextRequest;
use aidp_memory::{
    Embedder, IndexOptions, IndexRoot, Indexer, KnowledgeContextProvider, MemoryStore, ProjectRef,
    Result, Source,
};
use std::path::{Path, PathBuf};

const PROJECT: &str = "mini";
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/knowledge");
const UNITY_AGENT: &str = "unity-issue-implementer";
const PIXEL_AGENT: &str = "pixel-artist";
const MODEL: &str = "fake-model";
const FAKE_DIM: usize = 4;
const BUDGET: usize = 1500;
const COYOTE_NOTE: &str = "coyote-time.md";
const OUTLINE_NOTE: &str = "outline.md";
const HEADING: &str = "### Extraits pertinents";

#[derive(serde::Serialize)]
struct RegisteredProject {
    id: &'static str,
    name: &'static str,
    path: String,
}

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
    gateway: tempfile::TempDir,
    vault: tempfile::TempDir,
    workspace: tempfile::TempDir,
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

fn setup() -> Setup {
    let setup = Setup {
        gateway: tempfile::tempdir().unwrap(),
        vault: tempfile::tempdir().unwrap(),
        workspace: tempfile::tempdir().unwrap(),
    };
    copy_tree(Path::new(FIXTURE), setup.vault.path());
    let registry = vec![RegisteredProject {
        id: "mini-id",
        name: PROJECT,
        path: setup.workspace.path().to_string_lossy().to_string(),
    }];
    std::fs::write(
        setup.gateway.path().join("projects.json"),
        serde_json::to_string(&registry).unwrap(),
    )
    .unwrap();
    let project = ProjectRef {
        key: PROJECT.into(),
        root: setup.workspace.path().to_path_buf(),
    };
    let options = IndexOptions {
        sources: vec![Source::Knowledge],
        full: false,
        extra_roots: vec![IndexRoot {
            source: Source::Knowledge,
            dir: setup.vault.path().join("Knowledge/Mini"),
            prefix: "Knowledge/Mini".to_string(),
        }],
        global_knowledge: Some(IndexRoot {
            source: Source::Knowledge,
            dir: setup.vault.path().join("Knowledge/_global"),
            prefix: "Knowledge/_global".to_string(),
        }),
    };
    let store = MemoryStore::open_in(setup.gateway.path()).unwrap();
    Indexer::new(&store).index(&project, &options).unwrap();
    setup
}

fn request(setup: &Setup, agent: &str, text: &str) -> StageContextRequest {
    StageContextRequest {
        project_root: setup.workspace.path().to_path_buf(),
        stage_id: "implement".into(),
        stage_description: text.into(),
        agent_type: agent.into(),
        route: Some(KnowledgeRoute::Code),
        issue: None,
        issue_text: None,
        budget_tokens: BUDGET,
    }
}

fn provider(setup: &Setup) -> KnowledgeContextProvider {
    KnowledgeContextProvider::new(setup.gateway.path())
        .with_vault(Some(setup.vault.path().to_path_buf()))
        .lexical()
}

#[test]
fn the_block_holds_the_notes_meant_for_the_agent() {
    let setup = setup();
    let block = provider(&setup)
        .stage_context(&request(&setup, UNITY_AGENT, "coyote saut plateforme"))
        .expect("a block");
    assert!(block.contains(HEADING), "{block}");
    assert!(block.contains(COYOTE_NOTE), "{block}");
    assert!(
        !block.lines().any(|line| line.starts_with("## ")),
        "{block}"
    );
}

#[test]
fn another_agent_does_not_get_those_notes() {
    let setup = setup();
    let block = provider(&setup)
        .stage_context(&request(&setup, PIXEL_AGENT, "coyote saut plateforme"))
        .unwrap_or_default();
    assert!(!block.contains(COYOTE_NOTE), "{block}");
}

#[test]
fn the_block_is_none_when_nothing_matches() {
    let setup = setup();
    let mut unmatched = request(&setup, "zzzzagent", "zzzzqqqq");
    unmatched.stage_id = "zzzzwwww".into();
    unmatched.route = None;
    assert!(provider(&setup).stage_context(&unmatched).is_none());
}

#[test]
fn an_embedder_switches_to_the_hybrid_retriever_and_still_answers() {
    let setup = setup();
    let hybrid = KnowledgeContextProvider::new(setup.gateway.path())
        .with_vault(Some(setup.vault.path().to_path_buf()))
        .with_embedder_loader(|_| Some(Box::new(ConstantEmbedder)));
    let block = hybrid
        .stage_context(&request(&setup, UNITY_AGENT, "coyote saut plateforme"))
        .expect("a block");
    assert!(block.contains(COYOTE_NOTE), "{block}");
}

#[test]
fn an_unreadable_gateway_gives_no_block_instead_of_an_error() {
    let setup = setup();
    let blocked: PathBuf = setup.gateway.path().join("projects.json");
    let broken = KnowledgeContextProvider::new(blocked).lexical();
    assert!(broken
        .stage_context(&request(&setup, UNITY_AGENT, "coyote"))
        .is_none());
}

#[test]
fn the_pixel_agent_reads_its_own_notes() {
    let setup = setup();
    let block = provider(&setup)
        .stage_context(&request(&setup, PIXEL_AGENT, "contour selectif sprite"))
        .expect("a block");
    assert!(block.contains(OUTLINE_NOTE), "{block}");
}
