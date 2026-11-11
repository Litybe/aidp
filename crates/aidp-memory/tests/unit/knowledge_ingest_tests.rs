use super::*;
use crate::embedder::Embedder;
use crate::indexer::Indexer;
use crate::models::index_options::IndexOptions;
use crate::models::index_root::IndexRoot;
use crate::models::project_ref::ProjectRef;
use serde_json::{json, Value};

const REPO: &str = "Mini";
const PROJECT: &str = "mini";
const RUN: &str = "run:r-1";
const TODAY: &str = "2026-11-11";
const MODEL: &str = "scripted";
const EXISTING: &str = "Knowledge/_global/unity/coyote-time.md";
const EXISTING_BODY: &str =
    "Le temps de grace au saut laisse sauter juste apres avoir quitte une plateforme pendant quelques frames.";
const NOTE: &str = "---\ntype: note\ndomain: unity\ntags: [movement]\nagents: []\nstatus: active\nimportance: 0.8\nupdated: 2026-01-01\nsource: skill:x\n---\n# Coyote time\n\nLe temps de grace au saut laisse sauter juste apres avoir quitte une plateforme pendant quelques frames.\n\n## Evidence\n- 2026-01-01 skill:x\n";

struct ScriptedEmbedder;

impl Embedder for ScriptedEmbedder {
    fn model_id(&self) -> &str {
        MODEL
    }

    fn dim(&self) -> usize {
        2
    }

    fn embed(&self, texts: &[&str]) -> crate::error::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| {
                if text.contains("zz-same") {
                    vec![0.96, 0.28]
                } else if text.contains("zz-near") {
                    vec![0.8, 0.6]
                } else if text.contains("zz-origin") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }
}

struct Fixture {
    store: MemoryStore,
    vault: tempfile::TempDir,
    workspace: tempfile::TempDir,
}

impl Fixture {
    fn new(notes: &[(&str, &str)]) -> Self {
        let fixture = Self {
            store: MemoryStore::open_in_memory().unwrap(),
            vault: tempfile::tempdir().unwrap(),
            workspace: tempfile::tempdir().unwrap(),
        };
        for (rel, content) in notes {
            let path = fixture.vault.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        fixture
    }

    fn index(&self, embedder: Option<&dyn Embedder>) {
        let project = ProjectRef {
            key: PROJECT.into(),
            root: self.workspace.path().to_path_buf(),
        };
        let options = IndexOptions {
            sources: vec![Source::Knowledge],
            full: false,
            extra_roots: vec![IndexRoot {
                source: Source::Knowledge,
                dir: self.vault.path().join("Knowledge").join(REPO),
                prefix: format!("Knowledge/{REPO}"),
            }],
            global_knowledge: Some(IndexRoot {
                source: Source::Knowledge,
                dir: self.vault.path().join("Knowledge/_global"),
                prefix: "Knowledge/_global".into(),
            }),
        };
        let mut indexer = Indexer::new(&self.store);
        if let Some(embedder) = embedder {
            indexer = indexer.with_embedder(embedder);
        }
        indexer.index(&project, &options).unwrap();
    }

    fn ingestor(&self) -> KnowledgeIngestor<'_> {
        KnowledgeIngestor::new(&self.store, self.vault.path(), REPO, PROJECT).with_today(TODAY)
    }

    fn ingest(&self, proposals: Vec<Value>) -> KnowledgeIngestReport {
        self.ingest_from(RUN, proposals)
    }

    fn ingest_from(&self, source: &str, proposals: Vec<Value>) -> KnowledgeIngestReport {
        self.ingestor()
            .ingest_bytes(
                "file.json",
                json!({ "proposals": proposals }).to_string().as_bytes(),
                source,
            )
            .unwrap()
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.vault.path().join(rel)).unwrap()
    }

    fn exists(&self, rel: &str) -> bool {
        self.vault.path().join(rel).exists()
    }
}

fn proposal(overrides: Value) -> Value {
    let mut base = json!({
        "action": "add", "domain": "unity", "scope": "project",
        "slug": "new-rule", "title": "Nouvelle regle", "body": "Une regle sur les shaders et les passes.",
        "evidence": "vu dans le run"
    });
    for (key, value) in overrides.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

#[test]
fn a_fresh_project_note_is_written_with_the_existing_frontmatter_format() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    fixture.index(None);
    let report = fixture.ingest(vec![proposal(json!({
        "tags": ["shader"], "agents": ["executor"], "importance": 0.7
    }))]);
    assert_eq!((report.files, report.added), (1, 1));
    let path = "Knowledge/Mini/notes/unity/new-rule.md";
    assert_eq!(report.results[0].path, path);
    assert_eq!(
        fixture.read(path),
        "---\ntype: note\ndomain: unity\ntags: [shader]\nagents: [executor]\nstatus: active\nimportance: 0.7\nupdated: 2026-11-11\nsource: run:r-1\n---\n# Nouvelle regle\n\nUne regle sur les shaders et les passes.\n\n## Evidence\n- 2026-11-11 run:r-1: vu dans le run\n"
    );
}

#[test]
fn a_global_note_goes_under_the_global_folder() {
    let fixture = Fixture::new(&[]);
    let report = fixture.ingest(vec![proposal(json!({ "scope": "global" }))]);
    assert_eq!(report.added, 1);
    assert!(fixture.exists("Knowledge/_global/unity/new-rule.md"));
}

#[test]
fn without_a_model_a_near_identical_note_only_gains_evidence() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    fixture.index(None);
    let report = fixture.ingest(vec![proposal(json!({
        "slug": "coyote-again", "title": "Coyote time", "body": EXISTING_BODY, "evidence": "revu en run"
    }))]);
    assert_eq!((report.merged, report.added), (1, 0));
    assert_eq!(report.results[0].path, EXISTING);
    let merged = fixture.read(EXISTING);
    assert!(merged.contains("- 2026-01-01 skill:x\n- 2026-11-11 run:r-1: revu en run\n"));
    assert!(merged.contains("updated: 2026-11-11"));
    assert!(!fixture.exists("Knowledge/Mini/notes/unity/coyote-again.md"));
}

#[test]
fn without_a_model_an_unrelated_note_is_added() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    fixture.index(None);
    let report = fixture.ingest(vec![proposal(json!({}))]);
    assert_eq!(report.added, 1);
}

#[test]
fn a_cosine_over_the_merge_threshold_adds_evidence() {
    let fixture = Fixture::new(&[(
        EXISTING,
        &NOTE.replace("# Coyote time", "# Coyote time zz-origin"),
    )]);
    fixture.index(Some(&ScriptedEmbedder));
    let ingestor = fixture.ingestor().with_embedder(&ScriptedEmbedder);
    let body = json!({ "proposals": [proposal(json!({ "body": "zz-same duplicate" }))] });
    let report = ingestor
        .ingest_bytes("f.json", body.to_string().as_bytes(), RUN)
        .unwrap();
    assert_eq!((report.merged, report.inboxed, report.added), (1, 0, 0));
    assert!(fixture.read(EXISTING).contains("run:r-1: vu dans le run"));
}

#[test]
fn a_cosine_between_the_thresholds_goes_to_the_inbox_as_a_candidate() {
    let fixture = Fixture::new(&[(
        EXISTING,
        &NOTE.replace("# Coyote time", "# Coyote time zz-origin"),
    )]);
    fixture.index(Some(&ScriptedEmbedder));
    let ingestor = fixture.ingestor().with_embedder(&ScriptedEmbedder);
    let body = json!({ "proposals": [proposal(json!({ "body": "zz-near variante" }))] });
    let report = ingestor
        .ingest_bytes("f.json", body.to_string().as_bytes(), RUN)
        .unwrap();
    assert_eq!((report.inboxed, report.merged, report.added), (1, 0, 0));
    assert_eq!(report.results[0].path, "Knowledge/_inbox/new-rule.md");
    let inbox = fixture.read("Knowledge/_inbox/new-rule.md");
    assert!(inbox.contains("status: proposed\n"));
    assert!(inbox.contains("candidate_of: \"[[Knowledge/_global/unity/coyote-time]]\"\n"));
    assert!(inbox.contains("action: add\nscope: project\nrepo: Mini\n"));
    assert!(!fixture.exists("Knowledge/Mini/notes/unity/new-rule.md"));
}

#[test]
fn a_cosine_under_the_inbox_threshold_adds_the_note() {
    let fixture = Fixture::new(&[(
        EXISTING,
        &NOTE.replace("# Coyote time", "# Coyote time zz-origin"),
    )]);
    fixture.index(Some(&ScriptedEmbedder));
    let ingestor = fixture.ingestor().with_embedder(&ScriptedEmbedder);
    let body = json!({ "proposals": [proposal(json!({}))] });
    let report = ingestor
        .ingest_bytes("f.json", body.to_string().as_bytes(), RUN)
        .unwrap();
    assert_eq!(report.added, 1);
}

#[test]
fn a_model_without_stored_vectors_falls_back_to_word_overlap() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    fixture.index(None);
    let ingestor = fixture.ingestor().with_embedder(&ScriptedEmbedder);
    let body = json!({ "proposals": [proposal(json!({
        "title": "Coyote time", "body": EXISTING_BODY
    }))] });
    let report = ingestor
        .ingest_bytes("f.json", body.to_string().as_bytes(), RUN)
        .unwrap();
    assert_eq!(report.merged, 1);
}

#[test]
fn lore_game_design_and_rooms_always_wait_in_the_inbox() {
    let fixture = Fixture::new(&[]);
    let report = fixture.ingest(vec![
        proposal(json!({ "domain": "lore", "slug": "a-lore" })),
        proposal(json!({ "domain": "game-design", "slug": "a-design" })),
        proposal(json!({ "domain": "rooms", "slug": "a-room", "scope": "global" })),
    ]);
    assert_eq!((report.inboxed, report.added), (3, 0));
    for slug in ["a-lore", "a-design", "a-room"] {
        assert!(
            fixture.exists(&format!("Knowledge/_inbox/{slug}.md")),
            "{slug}"
        );
    }
}

#[test]
fn an_existing_destination_is_never_overwritten() {
    let fixture = Fixture::new(&[("Knowledge/Mini/notes/unity/new-rule.md", NOTE)]);
    let report = fixture.ingest(vec![proposal(
        json!({ "body": "Autre texte entierement different." }),
    )]);
    assert_eq!(report.inboxed, 1);
    assert_eq!(fixture.read("Knowledge/Mini/notes/unity/new-rule.md"), NOTE);
    assert!(fixture
        .read("Knowledge/_inbox/new-rule.md")
        .contains("candidate_of: \"[[Knowledge/Mini/notes/unity/new-rule]]\""));
}

#[test]
fn two_inbox_notes_with_one_slug_keep_both() {
    let fixture = Fixture::new(&[]);
    fixture.ingest_from("run:r-1", vec![proposal(json!({ "domain": "lore" }))]);
    fixture.ingest_from("run:r-2", vec![proposal(json!({ "domain": "lore" }))]);
    assert!(fixture.exists("Knowledge/_inbox/new-rule.md"));
    assert!(fixture.exists("Knowledge/_inbox/new-rule-2.md"));
}

#[test]
fn an_update_rewrites_the_body_and_keeps_the_previous_version() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    let report = fixture.ingest(vec![proposal(json!({
        "action": "update", "scope": "global", "slug": "coyote-time",
        "title": "Coyote time v2", "body": "Nouvelle version du texte.",
        "target": "_global/unity/coyote-time.md", "evidence": "mesure"
    }))]);
    assert_eq!((report.updated, report.added), (1, 0));
    let rewritten = fixture.read(EXISTING);
    assert!(rewritten.starts_with("---\ntype: note\ndomain: unity\ntags: [movement]\n"));
    assert!(rewritten.contains("updated: 2026-11-11\nsource: run:r-1\n---\n# Coyote time v2\n\nNouvelle version du texte.\n"));
    assert!(rewritten.contains("## Evidence\n- 2026-01-01 skill:x\n- 2026-11-11 run:r-1: mesure\n"));
    let history = fixture.read("Knowledge/_history/_global__unity__coyote-time__2026-01-01.md");
    assert_eq!(history, NOTE);
}

#[test]
fn a_second_update_the_same_day_keeps_both_previous_versions() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    let update = proposal(json!({
        "action": "update", "target": "Knowledge/_global/unity/coyote-time.md"
    }));
    fixture.ingest_from("run:r-1", vec![update.clone()]);
    fixture.ingest_from("run:r-2", vec![update]);
    let count = std::fs::read_dir(fixture.vault.path().join("Knowledge/_history"))
        .unwrap()
        .count();
    assert_eq!(count, 2);
}

#[test]
fn an_update_of_a_protected_note_waits_in_the_inbox() {
    let lore = NOTE.replace("domain: unity", "domain: lore");
    let fixture = Fixture::new(&[("Knowledge/_global/lore/old.md", &lore)]);
    let report = fixture.ingest(vec![proposal(json!({
        "action": "update", "target": "_global/lore/old.md"
    }))]);
    assert_eq!(report.inboxed, 1);
    assert_eq!(fixture.read("Knowledge/_global/lore/old.md"), lore);
    let inbox = fixture.read("Knowledge/_inbox/new-rule.md");
    assert!(inbox.contains("action: update\n"));
    assert!(inbox.contains("target: \"[[Knowledge/_global/lore/old]]\"\n"));
}

#[test]
fn a_supersede_writes_the_new_note_and_marks_the_old_one() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    let report = fixture.ingest(vec![proposal(json!({
        "action": "supersede", "scope": "global", "slug": "coyote-time-2",
        "target": "_global/unity/coyote-time.md"
    }))]);
    assert_eq!(report.superseded, 1);
    assert_eq!(
        report.results[0].path,
        "Knowledge/_global/unity/coyote-time-2.md"
    );
    assert!(fixture.exists("Knowledge/_global/unity/coyote-time-2.md"));
    let old = fixture.read(EXISTING);
    assert!(old.contains("status: superseded\n"));
    assert!(old.contains("superseded_by: \"[[Knowledge/_global/unity/coyote-time-2]]\"\n"));
    assert!(old.contains("updated: 2026-11-11"));
    assert!(old.contains("# Coyote time"));
}

#[test]
fn an_invalid_file_creates_nothing_and_is_reported() {
    let fixture = Fixture::new(&[]);
    let many: Vec<Value> = (0..6).map(|_| proposal(json!({}))).collect();
    let report = fixture.ingest(many);
    assert_eq!((report.files, report.changed()), (0, 0));
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(report.rejected[0].file, "file.json");
    assert!(!fixture.exists("Knowledge"));
}

#[test]
fn a_bad_target_rejects_only_its_proposal() {
    let fixture = Fixture::new(&[]);
    let report = fixture.ingest(vec![
        proposal(json!({ "action": "update", "target": "_global/unity/missing.md" })),
        proposal(
            json!({ "action": "update", "target": "Other/notes/unity/x.md", "slug": "other" }),
        ),
        proposal(json!({ "slug": "fine" })),
    ]);
    assert_eq!(
        (report.files, report.added, report.rejected.len()),
        (1, 1, 2)
    );
    assert_eq!(report.rejected[0].file, "file.json#new-rule");
    assert!(report.rejected[0].reason.contains("target note not found"));
    assert_eq!(report.rejected[1].file, "file.json#other");
}

#[test]
fn a_file_is_read_from_disk() {
    let fixture = Fixture::new(&[]);
    let file = fixture.workspace.path().join("knowledge-proposals.json");
    std::fs::write(
        &file,
        json!({ "proposals": [proposal(json!({}))] }).to_string(),
    )
    .unwrap();
    let report = fixture.ingestor().ingest_file(&file, "manual").unwrap();
    assert_eq!(report.added, 1);
    assert!(fixture
        .read("Knowledge/Mini/notes/unity/new-rule.md")
        .contains("source: manual\n"));
}

#[test]
fn the_report_adds_up() {
    let mut total = KnowledgeIngestReport::default();
    let fixture = Fixture::new(&[]);
    total.absorb(fixture.ingest(vec![proposal(json!({}))]));
    total.absorb(fixture.ingest(vec![proposal(json!({ "domain": "lore", "slug": "x" }))]));
    assert_eq!((total.files, total.added, total.inboxed), (2, 1, 1));
    assert_eq!(total.changed(), 2);
    assert_eq!(total.results.len(), 2);
}

#[test]
fn labels_with_a_colon_are_quoted_and_the_note_parses_back() {
    let fixture = Fixture::new(&[]);
    let report = fixture.ingest(vec![proposal(json!({
        "tags": ["scope:ui", "plain"], "agents": ["plugin:agent"]
    }))]);
    assert_eq!(report.added, 1);
    let written = fixture.read("Knowledge/Mini/notes/unity/new-rule.md");
    assert!(written.contains("tags: [\"scope:ui\", plain]\n"));
    let parsed = parse_knowledge(&written, "x");
    assert_eq!(parsed.front.tags, ["scope:ui", "plain"]);
    assert_eq!(parsed.front.agents, ["plugin:agent"]);
}

#[test]
fn replaying_a_file_changes_nothing() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    let proposals = vec![
        proposal(json!({})),
        proposal(json!({ "slug": "lore-rule", "domain": "lore" })),
        proposal(json!({
            "action": "update", "target": EXISTING, "slug": "upd", "body": "Corps neuf."
        })),
    ];
    let first = fixture.ingest(proposals.clone());
    assert_eq!((first.added, first.inboxed, first.updated), (1, 1, 1));
    let note_after = fixture.read("Knowledge/Mini/notes/unity/new-rule.md");
    let target_after = fixture.read(EXISTING);
    let count = |dir: &str| {
        std::fs::read_dir(fixture.vault.path().join(dir))
            .unwrap()
            .count()
    };
    let history = count("Knowledge/_history");
    fixture.ingest(proposals);
    assert_eq!(
        fixture.read("Knowledge/Mini/notes/unity/new-rule.md"),
        note_after
    );
    assert_eq!(fixture.read(EXISTING), target_after);
    assert_eq!(count("Knowledge/_inbox"), 1);
    assert_eq!(count("Knowledge/_history"), history);
}

#[test]
fn a_merge_replay_does_not_repeat_the_evidence() {
    let fixture = Fixture::new(&[(EXISTING, NOTE)]);
    fixture.index(None);
    let merge = proposal(json!({
        "slug": "coyote-again", "title": "Coyote time", "body": EXISTING_BODY, "evidence": "revu"
    }));
    fixture.ingest(vec![merge.clone()]);
    let once = fixture.read(EXISTING);
    fixture.ingest(vec![merge]);
    assert_eq!(fixture.read(EXISTING), once);
}

#[test]
fn a_failing_proposal_is_rejected_and_the_next_ones_still_apply() {
    let fixture = Fixture::new(&[]);
    let report = fixture.ingest(vec![
        proposal(json!({ "action": "update", "target": "_global/unity/ghost.md", "slug": "bad" })),
        proposal(json!({})),
    ]);
    assert_eq!((report.rejected.len(), report.added), (1, 1));
    assert!(report.rejected[0].file.ends_with("#bad"));
}

#[test]
fn update_targets_outside_the_notes_of_this_repo_are_refused() {
    let other = "Knowledge/Other/notes/unity/x.md";
    let history = "Knowledge/_history/old.md";
    let hidden = "Knowledge/_global/_secret/x.md";
    let text = "Knowledge/_global/unity/x.txt";
    let fixture = Fixture::new(&[
        (other, NOTE),
        (history, NOTE),
        (hidden, NOTE),
        (text, NOTE),
        (EXISTING, NOTE),
    ]);
    for target in [other, history, hidden, text] {
        let report = fixture.ingest(vec![proposal(json!({
            "action": "update", "target": target, "slug": "upd", "body": "Neuf."
        }))]);
        assert_eq!((report.updated, report.rejected.len()), (0, 1), "{target}");
        assert_eq!(fixture.read(target), NOTE);
    }
    let allowed = fixture.ingest(vec![proposal(json!({
        "action": "update", "target": EXISTING, "slug": "upd", "body": "Neuf."
    }))]);
    assert_eq!(allowed.updated, 1);
}

#[test]
fn evidence_text_is_kept_on_one_line() {
    let fixture = Fixture::new(&[]);
    fixture.ingest(vec![proposal(json!({
        "evidence": "line one\n## Evidence\n- fake"
    }))]);
    let written = fixture.read("Knowledge/Mini/notes/unity/new-rule.md");
    assert_eq!(crate::note_text::evidence_of(&written).len(), 1);
    assert_eq!(written.matches("## Evidence").count(), 2);
}
