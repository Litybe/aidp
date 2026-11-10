use super::*;
use crate::models::constants::limits::{TEXT_MAX_CHARS, TITLE_MAX_CHARS};
use crate::models::constants::proposal_fields::TRUNCATION_MARK;
use crate::models::enums::Route;
use crate::store::MemoryStore;

const PROJECT: &str = "pixelhood";

const TOPICS: [(&str, &str); 6] = [
    (
        "Rigidbody en FixedUpdate",
        "Appliquer la vitesse du corps rigide uniquement pendant FixedUpdate.",
    ),
    (
        "Camera amortie",
        "Lisser le suivi de la camera avec un amortissement exponentiel.",
    ),
    (
        "Pool de particules",
        "Reutiliser les systemes de particules plutot que les instancier.",
    ),
    (
        "Sauvegarde atomique",
        "Ecrire le fichier de sauvegarde dans un temporaire puis le renommer.",
    ),
    (
        "Audio mixer",
        "Router chaque source sonore vers un groupe du mixer.",
    ),
    (
        "Tilemap collisions",
        "Fusionner les collisions de la tilemap avec un composite collider.",
    ),
];

fn proposal_json(count: usize) -> String {
    let items: Vec<String> = TOPICS
        .iter()
        .take(count)
        .enumerate()
        .map(|(n, (title, text))| {
            format!(
                r#"{{"kind":"lesson","title":"{title}","text":"{text}","scope":{{"route":"code","paths":["Assets/**"],"tags":["t"]}},"refs":[{{"type":"issue","id":"GH-{n}"}}],"confidence":0.8}}"#
            )
        })
        .collect();
    format!(r#"{{"proposals":[{}]}}"#, items.join(","))
}

#[test]
fn confident_proposals_are_active_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("memory-proposals.json");
    std::fs::write(&file, proposal_json(3)).unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let report = manager.ingest(PROJECT, &file).unwrap();
    assert_eq!(
        (
            report.files,
            report.created,
            report.deduplicated,
            report.activated
        ),
        (1, 3, 0, 3)
    );
    assert!(report.rejected.is_empty());
    let all = manager.list(PROJECT, &Default::default()).unwrap();
    assert!(all.iter().all(|m| m.status == MemoryStatus::Active));
}

#[test]
fn an_unsure_proposal_stays_proposed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("memory-proposals.json");
    std::fs::write(&file, proposal_json(1).replace("0.8", "0.5")).unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    assert_eq!(manager.ingest(PROJECT, &file).unwrap().activated, 0);
    let all = manager.list(PROJECT, &Default::default()).unwrap();
    assert_eq!(all[0].status, MemoryStatus::Proposed);
}

#[test]
fn more_than_five_proposals_is_refused_entirely() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("memory-proposals.json");
    std::fs::write(&file, proposal_json(6)).unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let report = manager.ingest(PROJECT, &file).unwrap();
    assert_eq!(report.created, 0);
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].reason.contains("at most 5"));
    assert!(manager
        .list(PROJECT, &Default::default())
        .unwrap()
        .is_empty());
}

#[test]
fn unknown_fields_bad_kind_and_oversized_text_are_refused() {
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        r#"{"proposals":[{"kind":"lesson","title":"t","text":"x","extra":1}]}"#.to_string(),
        r#"{"proposals":[{"kind":"wisdom","title":"t","text":"x"}]}"#.to_string(),
        format!(
            r#"{{"proposals":[{{"kind":"fact","title":"t","text":"{}"}}]}}"#,
            "y".repeat(601)
        ),
        r#"{"proposals":[],"other":true}"#.to_string(),
        "not json".to_string(),
    ];
    for (index, content) in cases.iter().enumerate() {
        let file = dir
            .path()
            .join(format!("case{index}"))
            .join("memory-proposals.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, content).unwrap();
        let report = manager.ingest(PROJECT, &file).unwrap();
        assert_eq!(report.rejected.len(), 1, "case {index}");
        assert_eq!(report.created, 0, "case {index}");
    }
}

const RUN_RELATIVE: &str = "GH-321/20261109-173201-7a10/test";
const ISSUE_ID: &str = "GH-321";
const RUN_ID: &str = "20261109-173201-7a10";
const FEEDBACK_SHAPE: &str = r#"{"proposals":[{"type":"feedback","name":"slug-name","description":"Verifier le meta genere","body":"Le meta copie garde le nom du modele."}]}"#;
const LESSON_SHAPE: &str = r#"{"proposals":[{"title":"Using Is","lesson":"Alias Is obligatoire.","evidence":"AttackTokenTests.cs:262"}]}"#;
const FACT_SHAPE: &str = r#"{"proposals":[{"type":"project","title":"Meta caches","fact":"Attribut Hidden sous Windows.","why":"Ecriture refusee.","how_to_apply":"Retirer l attribut."}]}"#;
const RECIPE_SHAPE: &str =
    r#"{"proposals":[{"type":"recipe","title":"Lire un meta","body":"Lire par regex."}]}"#;
const NO_TEXT_SHAPE: &str = r#"{"proposals":[{"type":"feedback","title":"Sans contenu"}]}"#;
const SECOND_INVALID: &str = r#"{"proposals":[{"kind":"lesson","title":"Premier","text":"Valide."},{"kind":"lesson","title":"Second","text":"Hors bornes.","confidence":3.0}]}"#;

fn ingest_one(manager: &MemoryManager<'_>, root: &Path, content: &str) -> IngestReport {
    let folder = root.join(RUN_RELATIVE);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join(PROPOSALS_FILE);
    std::fs::write(&file, content).unwrap();
    manager.ingest(PROJECT, &file).unwrap()
}

#[test]
fn undocumented_shapes_are_folded_into_the_documented_one() {
    let cases = [
        (
            FEEDBACK_SHAPE,
            MemoryKind::Lesson,
            "Verifier le meta genere",
            "Le meta copie garde le nom du modele.",
        ),
        (
            LESSON_SHAPE,
            MemoryKind::Lesson,
            "Using Is",
            "Alias Is obligatoire.\nEvidence: AttackTokenTests.cs:262",
        ),
        (
            FACT_SHAPE,
            MemoryKind::Fact,
            "Meta caches",
            "Attribut Hidden sous Windows.\nWhy: Ecriture refusee.\nHow: Retirer l attribut.",
        ),
        (
            RECIPE_SHAPE,
            MemoryKind::Lesson,
            "Lire un meta",
            "Lire par regex.",
        ),
    ];
    for (content, kind, title, text) in cases {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open_in_memory().unwrap();
        let manager = MemoryManager::new(&store);
        let report = ingest_one(&manager, dir.path(), content);
        assert_eq!((report.created, report.rejected.len()), (1, 0), "{title}");
        let memory = manager
            .list(PROJECT, &Default::default())
            .unwrap()
            .remove(0);
        assert_eq!(
            (memory.kind, memory.title.as_str(), memory.text.as_str()),
            (kind, title, text)
        );
    }
}

#[test]
fn an_undocumented_shape_is_shortened_to_the_limits() {
    let content = format!(
        r#"{{"proposals":[{{"type":"feedback","description":"{}","body":"{}"}}]}}"#,
        "t".repeat(200),
        "b".repeat(900)
    );
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    assert_eq!(ingest_one(&manager, dir.path(), &content).created, 1);
    let memory = manager
        .list(PROJECT, &Default::default())
        .unwrap()
        .remove(0);
    assert_eq!(memory.title.chars().count(), TITLE_MAX_CHARS);
    assert_eq!(memory.text.chars().count(), TEXT_MAX_CHARS);
    assert!(memory.text.ends_with(TRUNCATION_MARK));
}

#[test]
fn an_undocumented_shape_without_text_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let report = ingest_one(&manager, dir.path(), NO_TEXT_SHAPE);
    assert_eq!((report.created, report.rejected.len()), (0, 1));
}

#[test]
fn a_file_with_one_invalid_proposal_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let report = ingest_one(&manager, dir.path(), SECOND_INVALID);
    assert_eq!((report.created, report.rejected.len()), (0, 1));
    assert!(manager
        .list(PROJECT, &Default::default())
        .unwrap()
        .is_empty());
}

#[test]
fn a_proposal_without_route_takes_the_route_of_its_issue() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let folder = dir.path().join(RUN_RELATIVE);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join(PROPOSALS_FILE);
    std::fs::write(&file, LESSON_SHAPE).unwrap();
    let route_of = |issue: &str| (issue == ISSUE_ID).then_some(Route::Art);
    manager.ingest_routed(PROJECT, &file, &route_of).unwrap();
    let memory = manager
        .list(PROJECT, &Default::default())
        .unwrap()
        .remove(0);
    assert_eq!(memory.scope.route, Some(Route::Art));
}

#[test]
fn issue_and_run_are_read_from_the_vault_path() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    ingest_one(&manager, dir.path(), LESSON_SHAPE);
    let memory = manager
        .list(PROJECT, &Default::default())
        .unwrap()
        .remove(0);
    assert_eq!(
        memory.refs,
        vec![
            MemoryRef {
                ref_type: RefType::Issue,
                id: ISSUE_ID.to_string()
            },
            MemoryRef {
                ref_type: RefType::Run,
                id: RUN_ID.to_string()
            },
        ]
    );
}

#[test]
fn folder_is_scanned_and_repeats_are_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    for stage in ["review", "fix"] {
        let folder = dir.path().join(stage);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("memory-proposals.json"), proposal_json(1)).unwrap();
    }
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let report = manager.ingest(PROJECT, dir.path()).unwrap();
    assert_eq!(report.files, 2);
    assert_eq!(report.created, 1);
    assert_eq!(report.deduplicated, 1);
}

#[test]
fn missing_path_is_not_found() {
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let error = manager.ingest(PROJECT, Path::new("Z:/nope")).unwrap_err();
    assert!(matches!(error, MemoryError::NotFound(_)));
}

#[test]
fn file_with_a_utf8_bom_is_ingested() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("memory-proposals.json");
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(proposal_json(2).into_bytes());
    std::fs::write(&file, bytes).unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let report = manager.ingest(PROJECT, &file).unwrap();
    assert_eq!((report.created, report.rejected.len()), (2, 0));
}
