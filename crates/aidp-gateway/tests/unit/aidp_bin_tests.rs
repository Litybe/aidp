use super::*;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

fn fake_binary(dir: &Path, name: &str) -> PathBuf {
    let binary = dir.join(name);
    std::fs::write(&binary, b"").unwrap();
    binary
}

fn value_of<'a>(env: &'a [(String, String)], key: &str) -> &'a str {
    &env.iter().find(|(name, _)| name == key).unwrap().1
}

fn path_entries(env: &[(String, String)]) -> Vec<PathBuf> {
    std::env::split_paths(value_of(env, PATH_ENV)).collect()
}

fn joined(entries: &[PathBuf]) -> OsString {
    std::env::join_paths(entries).unwrap()
}

fn config_message(result: Result<Vec<(String, String)>, SessionError>) -> String {
    match result {
        Err(SessionError::Config(message)) => message,
        other => panic!("expected SessionError::Config, got {other:?}"),
    }
}

#[test]
fn path_without_binary_dir_gets_it_prepended_and_aidp_bin_is_current_exe() {
    let workspace = tempfile::tempdir().unwrap();
    let exe = fake_binary(workspace.path(), "aidp");
    let inherited = vec![PathBuf::from("/first"), PathBuf::from("/second")];

    let env = resolve_child_env(None, Ok(exe.clone()), Some(joined(&inherited))).unwrap();

    assert_eq!(value_of(&env, AIDP_BIN_ENV), exe.to_str().unwrap());
    let entries = path_entries(&env);
    assert_eq!(entries[0], workspace.path());
    assert_eq!(entries[1..], inherited[..]);
}

#[test]
fn user_set_aidp_bin_is_kept_and_its_dir_prefixes_path() {
    let user_dir = tempfile::tempdir().unwrap();
    let exe_dir = tempfile::tempdir().unwrap();
    let user_bin = fake_binary(user_dir.path(), "custom-aidp");
    let exe = fake_binary(exe_dir.path(), "aidp");

    let env = resolve_child_env(
        Some(user_bin.clone().into_os_string()),
        Ok(exe),
        Some(joined(&[PathBuf::from("/other")])),
    )
    .unwrap();

    assert_eq!(value_of(&env, AIDP_BIN_ENV), user_bin.to_str().unwrap());
    assert_eq!(path_entries(&env)[0], user_dir.path());
}

#[test]
fn empty_user_aidp_bin_falls_back_to_current_exe() {
    let workspace = tempfile::tempdir().unwrap();
    let exe = fake_binary(workspace.path(), "aidp");

    let env = resolve_child_env(Some(OsString::new()), Ok(exe.clone()), None).unwrap();

    assert_eq!(value_of(&env, AIDP_BIN_ENV), exe.to_str().unwrap());
}

#[test]
fn user_aidp_bin_pointing_at_a_missing_file_is_a_config_error_naming_the_path() {
    let workspace = tempfile::tempdir().unwrap();
    let exe = fake_binary(workspace.path(), "aidp");
    let missing = workspace.path().join("missing-aidp");

    let message = config_message(resolve_child_env(
        Some(missing.clone().into_os_string()),
        Ok(exe),
        None,
    ));

    assert!(message.contains(missing.to_str().unwrap()), "{message}");
    assert!(message.contains(AIDP_BIN_ENV), "{message}");
}

#[test]
fn relative_user_aidp_bin_is_a_config_error() {
    let workspace = tempfile::tempdir().unwrap();
    let exe = fake_binary(workspace.path(), "aidp");

    let message = config_message(resolve_child_env(
        Some(OsString::from("relative/aidp")),
        Ok(exe),
        None,
    ));

    assert!(message.contains("relative/aidp"), "{message}");
}

#[test]
fn current_exe_error_is_a_config_error_mentioning_aidp_bin() {
    let failure = io::Error::new(io::ErrorKind::NotFound, "no exe");

    let message = config_message(resolve_child_env(None, Err(failure), None));

    assert!(message.contains(AIDP_BIN_ENV), "{message}");
}

#[test]
fn current_exe_pointing_at_a_deleted_file_is_a_config_error_naming_the_path() {
    let workspace = tempfile::tempdir().unwrap();
    let deleted = workspace.path().join("gone-aidp");

    let message = config_message(resolve_child_env(None, Ok(deleted.clone()), None));

    assert!(message.contains(deleted.to_str().unwrap()), "{message}");
}

#[test]
fn binary_dir_already_first_in_path_is_not_duplicated() {
    let workspace = tempfile::tempdir().unwrap();
    let exe = fake_binary(workspace.path(), "aidp");
    let inherited = vec![workspace.path().to_path_buf(), PathBuf::from("/other")];

    let env = resolve_child_env(None, Ok(exe), Some(joined(&inherited))).unwrap();

    assert_eq!(path_entries(&env), inherited);
}

#[test]
fn missing_inherited_path_yields_only_the_binary_dir() {
    let workspace = tempfile::tempdir().unwrap();
    let exe = fake_binary(workspace.path(), "aidp");

    let env = resolve_child_env(None, Ok(exe), None).unwrap();

    assert_eq!(path_entries(&env), vec![workspace.path().to_path_buf()]);
}
