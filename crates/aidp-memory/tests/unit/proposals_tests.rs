use super::*;
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
fn valid_file_creates_proposed_memories() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("memory-proposals.json");
    std::fs::write(&file, proposal_json(3)).unwrap();
    let store = MemoryStore::open_in_memory().unwrap();
    let manager = MemoryManager::new(&store);
    let report = manager.ingest(PROJECT, &file).unwrap();
    assert_eq!(
        (report.files, report.created, report.deduplicated),
        (1, 3, 0)
    );
    assert!(report.rejected.is_empty());
    let all = manager.list(PROJECT, &Default::default()).unwrap();
    assert!(all.iter().all(|m| m.status == MemoryStatus::Proposed));
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
