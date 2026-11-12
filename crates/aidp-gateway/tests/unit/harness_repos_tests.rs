use super::*;
use aidp_testkit::TempWorkspace;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn clone_with_origin(ws: &TempWorkspace, name: &str, origin: &str) -> PathBuf {
    let dir = ws.child(name);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["remote", "add", "origin", origin]);
    dir
}

fn settings_with(entries: &[(&str, &Path)]) -> DoctorSettings {
    let mut config = DoctorConfig::default();
    for (repo, path) in entries {
        config
            .harness_repos
            .insert(repo.to_string(), path.to_string_lossy().to_string());
    }
    Arc::new(move || config.clone())
}

#[test]
fn a_configured_clone_with_a_matching_remote_resolves() {
    let ws = TempWorkspace::new("harness-repo-ok");
    let clone = clone_with_origin(&ws, "studio", "https://github.com/Litybe/aidp-studio.git");
    let resolve = resolver(settings_with(&[("Litybe/aidp-studio", &clone)]), None, None);
    assert_eq!(resolve("Litybe/aidp-studio"), Some(clone));
}

#[test]
fn the_repository_match_ignores_case() {
    let ws = TempWorkspace::new("harness-repo-case");
    let clone = clone_with_origin(&ws, "unity", "git@github.com:litybe/AIDP-UNITY.git");
    let resolve = resolver(settings_with(&[("Litybe/aidp-unity", &clone)]), None, None);
    assert_eq!(resolve("litybe/aidp-unity"), Some(clone));
    let config = DoctorConfig::default();
    assert_eq!(
        canonical_repo("litybe/AIDP", &config),
        Some("Litybe/aidp".to_string())
    );
    assert_eq!(canonical_repo("Litybe/other", &config), None);
}

#[test]
fn a_clone_whose_remote_names_another_repository_is_refused() {
    let ws = TempWorkspace::new("harness-repo-mismatch");
    let clone = clone_with_origin(&ws, "wrong", "https://github.com/Litybe/aidp-skill");
    let resolve = resolver(settings_with(&[("Litybe/aidp-studio", &clone)]), None, None);
    assert_eq!(resolve("Litybe/aidp-studio"), None);
}

#[test]
fn the_environment_clones_are_verified_too() {
    let ws = TempWorkspace::new("harness-repo-env");
    let good = clone_with_origin(&ws, "daemon", "ssh://git@github.com/Litybe/aidp.git");
    let bad = clone_with_origin(&ws, "skill", "https://github.com/Litybe/aidp");
    let resolve = resolver(settings_with(&[]), Some(good.clone()), Some(bad));
    assert_eq!(resolve("Litybe/aidp"), Some(good));
    assert_eq!(resolve("Litybe/aidp-skill"), None);
}

#[test]
fn a_config_entry_wins_over_the_environment() {
    let ws = TempWorkspace::new("harness-repo-wins");
    let env = clone_with_origin(&ws, "env", "https://github.com/Litybe/aidp.git");
    let configured = clone_with_origin(&ws, "configured", "https://github.com/Litybe/aidp.git");
    let resolve = resolver(
        settings_with(&[("Litybe/aidp", &configured)]),
        Some(env),
        None,
    );
    assert_eq!(resolve("Litybe/aidp"), Some(configured));
}

#[test]
fn unknown_missing_or_non_git_paths_resolve_to_nothing() {
    let ws = TempWorkspace::new("harness-repo-none");
    let plain = ws.child("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let missing = ws.child("missing");
    let resolve = resolver(
        settings_with(&[
            ("Litybe/aidp-studio", &plain),
            ("Litybe/aidp-unity", &missing),
        ]),
        None,
        None,
    );
    assert_eq!(resolve("Litybe/aidp-studio"), None);
    assert_eq!(resolve("Litybe/aidp-unity"), None);
    assert_eq!(resolve("Litybe/unknown"), None);
    assert_eq!(resolve("Litybe/aidp"), None);
}

#[test]
fn remote_urls_of_every_form_parse() {
    for url in [
        "https://github.com/Litybe/aidp.git",
        "https://user@github.com/Litybe/aidp/",
        "git@github.com:Litybe/aidp.git",
        "ssh://git@github.com/Litybe/aidp",
    ] {
        assert_eq!(remote_repo(url), Some("litybe/aidp".to_string()), "{url}");
    }
    assert_eq!(remote_repo("https://gitlab.com/Litybe/aidp"), None);
}

#[test]
fn doctor_settings_default_to_no_extra_repository() {
    assert!(DoctorConfig::default().harness_repos.is_empty());
}
