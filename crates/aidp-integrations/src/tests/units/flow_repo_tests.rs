use super::*;
use crate::models::constants::flow_repo::DEFAULT_WORKTREE_CAP;

const REPO_FLOW: &str = include_str!("fixtures/flow_repo/flow.json");
const USER_CONFIG: &str = include_str!("fixtures/flow_repo/user_config.json");
const EMPTY_OBJECT: &str = "{}";
const NOT_JSON: &str = "{ nope";
const ARRAY_JSON: &str = "[]";
const PARTIAL_REPO: &str = r#"{"caps":{"worktrees":4}}"#;
const REPO_WORKTREES: u32 = 5;
const USER_WORKTREES: u32 = 2;
const PARTIAL_WORKTREES: u32 = 4;

#[test]
fn flow_repo_defaults_apply_without_any_file() {
    let config = parse(None, None).unwrap();
    assert_eq!(config.caps.worktrees, DEFAULT_WORKTREE_CAP);
}

#[test]
fn flow_repo_reads_every_key_of_the_repo_file() {
    let config = parse(None, Some(REPO_FLOW)).unwrap();
    assert_eq!(config.caps.worktrees, REPO_WORKTREES);
}

#[test]
fn flow_repo_user_defaults_fill_what_the_repo_leaves_out() {
    let config = parse(Some(USER_CONFIG), Some(EMPTY_OBJECT)).unwrap();
    assert_eq!(config.caps.worktrees, USER_WORKTREES);
}

#[test]
fn flow_repo_repo_file_wins_over_user_defaults() {
    let config = parse(Some(USER_CONFIG), Some(REPO_FLOW)).unwrap();
    assert_eq!(config.caps.worktrees, REPO_WORKTREES);
}

#[test]
fn flow_repo_user_config_without_flow_key_changes_nothing() {
    let config = parse(Some(EMPTY_OBJECT), None).unwrap();
    assert_eq!(config, FlowRepoConfig::default());
}

#[test]
fn flow_repo_rejects_invalid_json_and_non_objects() {
    assert!(parse(None, Some(NOT_JSON)).is_err());
    assert!(parse(None, Some(ARRAY_JSON)).is_err());
    assert!(parse(Some(NOT_JSON), None).is_err());
}

#[test]
fn flow_repo_load_reads_the_workspace_file() {
    let workspace = std::env::temp_dir().join(format!("flow_repo_load_{}", std::process::id()));
    std::fs::create_dir_all(workspace.join(AIDP_DIR)).unwrap();
    std::fs::write(config_path(&workspace), PARTIAL_REPO).unwrap();
    let config = load(&workspace).unwrap();
    std::fs::remove_dir_all(&workspace).unwrap();
    assert_eq!(config.caps.worktrees, PARTIAL_WORKTREES);
}
