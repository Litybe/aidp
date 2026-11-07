use super::*;

#[test]
fn loggable_path_without_query_is_unchanged() {
    assert_eq!(loggable_path("/api/status"), "/api/status");
}

#[test]
fn loggable_path_drops_token() {
    assert_eq!(loggable_path("/api/events?token=secret"), "/api/events");
}

#[test]
fn loggable_path_keeps_whitelisted_params_only() {
    assert_eq!(
        loggable_path("/api/runs?limit=10&token=secret&status=done&x=1"),
        "/api/runs?limit=10&status=done"
    );
}

#[test]
fn loggable_path_keeps_encoded_question_mark_in_path() {
    assert_eq!(loggable_path("/api/a%3Fb?token=t"), "/api/a%3Fb");
}

#[test]
fn loggable_path_never_contains_token_value() {
    assert!(!loggable_path("/x?since=1&token=abc").contains("abc"));
}

#[test]
fn file_layer_creates_log_directory() {
    let dir = tempfile::tempdir().unwrap();
    let built = file_layer::<tracing_subscriber::Registry>(dir.path());
    assert!(built.is_ok());
    assert!(dir.path().join(LOG_DIR_NAME).is_dir());
}
