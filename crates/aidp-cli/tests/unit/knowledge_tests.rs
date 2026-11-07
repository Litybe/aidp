use super::*;

#[test]
fn stale_succeeds_on_a_missing_directory_and_writes_parsable_cache() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("fresh").join("knowledge");
    let ctx = Ctx::for_test(temp.path().to_path_buf(), None);
    let loc = KnowledgeLoc {
        knowledge_dir: Some(dir.clone()),
        run: None,
    };

    let output = stale(&ctx, &loc).unwrap();

    assert!(output.failure.is_none());
    let cache = std::fs::read_to_string(aidp_core::knowledge_stale::cache_path(&dir)).unwrap();
    serde_json::from_str::<serde_json::Value>(&cache).unwrap();
}
