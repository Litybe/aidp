use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;

use aidp_core::vault::{home_dir, resolve_global};

const VAULT_VARIABLE: &str = "AIDP_VAULT";
const HOME_VARIABLE: &str = "HOME";
const PROFILE_VARIABLE: &str = "USERPROFILE";
const TRACKED_VARIABLES: [&str; 3] = [VAULT_VARIABLE, HOME_VARIABLE, PROFILE_VARIABLE];

static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

/// Serializes environment access and restores the tracked variables on drop.
struct EnvironmentGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvironmentGuard {
    fn acquire() -> Self {
        let lock = ENVIRONMENT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = TRACKED_VARIABLES
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect();
        Self { saved, _lock: lock }
    }
}

impl Drop for EnvironmentGuard {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            match value {
                Some(original) => std::env::set_var(name, original),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn point_home_at(home: &std::path::Path) {
    std::env::set_var(HOME_VARIABLE, home);
    std::env::set_var(PROFILE_VARIABLE, home);
}

fn write_config(home: &std::path::Path, vault: &std::path::Path) {
    let config_dir = home.join(".aidp");
    std::fs::create_dir_all(&config_dir).unwrap();
    let body = serde_json::json!({ "vault": vault.to_string_lossy() });
    std::fs::write(config_dir.join("config.json"), body.to_string()).unwrap();
}

#[test]
fn disabled_values_resolve_to_none() {
    let _guard = EnvironmentGuard::acquire();
    let home = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    point_home_at(home.path());
    write_config(home.path(), vault.path());
    for value in ["off", "none", "0", "false", "-", "OFF"] {
        std::env::set_var(VAULT_VARIABLE, value);
        assert!(resolve_global().is_none(), "value {value}");
    }
}

#[test]
fn env_directory_resolves_to_that_path() {
    let _guard = EnvironmentGuard::acquire();
    let vault = tempfile::tempdir().unwrap();
    std::env::set_var(VAULT_VARIABLE, vault.path());
    let resolved = resolve_global().expect("vault");
    assert_eq!(resolved.root, vault.path());
    assert!(resolved.root.is_absolute());
}

#[test]
fn config_file_is_used_when_env_is_unset() {
    let _guard = EnvironmentGuard::acquire();
    let home = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    point_home_at(home.path());
    write_config(home.path(), vault.path());
    std::env::remove_var(VAULT_VARIABLE);
    let resolved = resolve_global().expect("vault");
    assert_eq!(resolved.root, vault.path());
}

#[test]
fn missing_config_resolves_to_none() {
    let _guard = EnvironmentGuard::acquire();
    let home = tempfile::tempdir().unwrap();
    point_home_at(home.path());
    std::env::remove_var(VAULT_VARIABLE);
    assert!(resolve_global().is_none());
}

#[test]
fn home_dir_prefers_user_profile_over_home() {
    let _guard = EnvironmentGuard::acquire();
    let profile = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::env::set_var(PROFILE_VARIABLE, profile.path());
    std::env::set_var(HOME_VARIABLE, home.path());
    assert_eq!(home_dir(), Some(PathBuf::from(profile.path())));
}
