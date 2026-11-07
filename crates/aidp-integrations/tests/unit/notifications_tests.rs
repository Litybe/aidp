use super::*;

fn write_config(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, text).unwrap();
    (dir, path)
}

#[test]
fn missing_file_enables_every_signal() {
    let dir = tempfile::tempdir().unwrap();
    let config = load_notifications_pr_from(&dir.path().join("absent.json"));
    assert_eq!(config, NotificationsPrConfig::default());
    assert!(config.new_comment && config.review_requested && config.conflict);
}

#[test]
fn one_disabled_signal_leaves_the_others_on() {
    let (_dir, path) = write_config(r#"{"notifications":{"pr":{"newComment":false}}}"#);
    let config = load_notifications_pr_from(&path);
    let expected = NotificationsPrConfig {
        new_comment: false,
        ..NotificationsPrConfig::default()
    };
    assert_eq!(config, expected);
}

#[test]
fn snake_case_alias_is_accepted() {
    let (_dir, path) = write_config(r#"{"notifications":{"pr":{"review_requested":false}}}"#);
    assert!(!load_notifications_pr_from(&path).review_requested);
}

/// A retired toggle left in an old config is ignored.
#[test]
fn retired_ci_toggles_are_ignored() {
    let (_dir, path) =
        write_config(r#"{"notifications":{"pr":{"ciRed":false,"sonar_red":false}}}"#);
    assert_eq!(
        load_notifications_pr_from(&path),
        NotificationsPrConfig::default()
    );
}

#[test]
fn malformed_json_enables_every_signal() {
    let (_dir, path) = write_config("{not json");
    assert_eq!(
        load_notifications_pr_from(&path),
        NotificationsPrConfig::default()
    );
}
