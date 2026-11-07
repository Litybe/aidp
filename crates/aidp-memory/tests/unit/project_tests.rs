use super::*;

#[test]
fn registered_name_wins() {
    let gateway = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let path = project.path().to_string_lossy().replace('\\', "/");
    std::fs::write(
        gateway.path().join("projects.json"),
        format!(r#"[{{"id":"p1","name":"pixelhood","path":"{path}"}}]"#),
    )
    .unwrap();
    let by_name = resolve_project(Some("pixelhood"), Some(gateway.path()), Path::new(".")).unwrap();
    assert_eq!(by_name.key, "pixelhood");
    let by_id = resolve_project(Some("p1"), Some(gateway.path()), Path::new(".")).unwrap();
    assert_eq!(by_id.key, "pixelhood");
    let by_path = resolve_project(
        Some(project.path().to_str().unwrap()),
        Some(gateway.path()),
        Path::new("."),
    )
    .unwrap();
    assert_eq!(by_path.key, "pixelhood");
}

#[test]
fn direct_path_uses_folder_name() {
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join("mygame");
    std::fs::create_dir(&dir).unwrap();
    let found = resolve_project(Some("mygame"), None, parent.path()).unwrap();
    assert_eq!(found.key, "mygame");
    assert_eq!(found.root, dir);
}

#[test]
fn unknown_project_is_not_found() {
    let parent = tempfile::tempdir().unwrap();
    let error = resolve_project(Some("nope"), None, parent.path()).unwrap_err();
    assert!(matches!(error, MemoryError::NotFound(_)));
}
