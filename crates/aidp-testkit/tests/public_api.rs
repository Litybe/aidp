//! Integration tests for aidp-testkit public API.
//!
//! These tests exercise the public API of aidp-testkit exclusively:
//! TempWorkspace, EnvGuard, snapshot_tree, normalize, ProcShim.

use aidp_testkit::{normalize, snapshot_tree, EnvGuard, TempWorkspace};
use std::fs;

/// Test 1: TempWorkspace + EnvGuard combination
/// - Create a TempWorkspace
/// - Set an environment variable through EnvGuard to the workspace path
/// - Verify the variable reads back correctly
/// - Drop both, verify the variable restored to prior value and directory is gone
#[test]
fn test_temp_workspace_and_env_guard_integration() {
    const TEST_VAR: &str = "AIDP_PUBLIC_API_TEST_WS_PATH";

    // Record the original value
    let original_value = std::env::var_os(TEST_VAR);

    // Create a workspace
    let ws = TempWorkspace::new("env_guard_test");
    let ws_path = ws.path().to_path_buf();

    // Verify the workspace exists
    assert!(ws_path.exists(), "Workspace should exist at {:?}", ws_path);

    // Set the environment variable to the workspace path through EnvGuard
    {
        let ws_path_str = ws_path.to_string_lossy().to_string();
        let _guard = EnvGuard::set(TEST_VAR, &ws_path_str);

        // Verify the variable was set
        let current = std::env::var(TEST_VAR).expect("variable should be set");
        assert_eq!(
            current, ws_path_str,
            "Variable should contain workspace path"
        );

        // Workspace should still exist
        assert!(
            ws_path.exists(),
            "Workspace should still exist while in scope"
        );
    }

    // After guard is dropped, verify the variable was restored
    let after_guard = std::env::var_os(TEST_VAR);
    assert_eq!(
        original_value, after_guard,
        "Environment variable should be restored to original value"
    );

    // Drop the workspace
    drop(ws);

    // Verify the workspace directory is gone
    assert!(
        !ws_path.exists(),
        "Workspace directory should be removed after drop, but found at {:?}",
        ws_path
    );
}

/// Test 2: snapshot_tree functionality
/// - Create a workspace and populate it with files
/// - Call snapshot_tree and verify it captures files with `/` separators
/// - Verify the map round-trips (serialization consistency)
#[test]
fn test_snapshot_tree_functionality() {
    let ws = TempWorkspace::new("snapshot_test");
    let base = ws.path();

    // Populate the workspace with a file structure
    fs::create_dir_all(base.join("dir_a").join("subdir")).expect("create directories");
    fs::write(base.join("dir_a/file1.txt"), b"content of file1").expect("write file1");
    fs::write(base.join("dir_a/subdir/file2.txt"), b"content of file2").expect("write file2");
    fs::write(base.join("root_file.txt"), b"root content").expect("write root file");

    // Take a snapshot
    let snap = snapshot_tree(base);

    // Verify files are captured
    assert!(
        snap.contains_key("dir_a/file1.txt"),
        "Should capture dir_a/file1.txt"
    );
    assert!(
        snap.contains_key("dir_a/subdir/file2.txt"),
        "Should capture nested file"
    );
    assert!(
        snap.contains_key("root_file.txt"),
        "Should capture root file"
    );

    // Verify keys use forward slashes (never backslashes on Windows)
    for key in snap.keys() {
        assert!(
            !key.contains('\\'),
            "Keys should use forward slashes, found backslash in: {}",
            key
        );
    }

    // Verify the map contents round-trip
    assert_eq!(
        snap.get("dir_a/file1.txt"),
        Some(&b"content of file1".to_vec())
    );
    assert_eq!(
        snap.get("dir_a/subdir/file2.txt"),
        Some(&b"content of file2".to_vec())
    );
    assert_eq!(snap.get("root_file.txt"), Some(&b"root content".to_vec()));

    // Take another snapshot and verify it's identical
    let snap2 = snapshot_tree(base);
    assert_eq!(
        snap, snap2,
        "Snapshots of identical content should be equal"
    );

    // Modify a file and take another snapshot
    fs::write(base.join("dir_a/file1.txt"), b"modified content").expect("modify file");
    let snap3 = snapshot_tree(base);

    // Verify the snapshot changed only for the modified file
    assert_ne!(snap, snap3, "Snapshot should differ after modification");
    assert_eq!(
        snap3.get("root_file.txt"),
        snap.get("root_file.txt"),
        "Unmodified files should remain unchanged"
    );
    assert_ne!(
        snap3.get("dir_a/file1.txt"),
        snap.get("dir_a/file1.txt"),
        "Modified file should differ"
    );
}

/// Test 3: normalize function
/// - Create a string with a workspace path and a run ID
/// - Apply normalize and verify workspace path becomes `<WS>`, run ID becomes `<RUN_ID>`
/// - Verify no backslashes remain
#[test]
fn test_normalize_functionality() {
    let ws = TempWorkspace::new("normalize_test");
    let ws_path = ws.path().to_string_lossy().to_string();

    // Create input with workspace path and run ID
    let run_id = "20260917-121500-a3f1";
    let input = format!(
        "Processed file at {} with run {}, backslash: C:\\Users\\test",
        ws_path, run_id
    );

    let normalized = normalize(&input, &ws);

    // Verify workspace path was replaced
    assert!(normalized.contains("<WS>"), "Should contain <WS> marker");
    assert!(
        !normalized.contains(&ws_path),
        "Normalized output should not contain workspace path"
    );

    // Verify run ID was replaced
    assert!(
        normalized.contains("<RUN_ID>"),
        "Should contain <RUN_ID> marker"
    );
    assert!(
        !normalized.contains(run_id),
        "Normalized output should not contain run ID"
    );

    // Verify no backslashes remain
    assert!(
        !normalized.contains('\\'),
        "Normalized output should contain no backslashes, got: {}",
        normalized
    );

    // Verify forward slashes are present (from the original path or the backslash conversion)
    assert!(
        normalized.contains('/'),
        "Normalized output should contain forward slashes"
    );
}
