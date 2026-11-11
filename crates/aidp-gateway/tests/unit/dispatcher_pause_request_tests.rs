use super::DispatcherPauseRequest;

#[test]
fn dispatcher_pause_empty_body_pauses() {
    let request: DispatcherPauseRequest = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(request.paused);
    assert!(request.actor.is_none());
}

#[test]
fn dispatcher_pause_explicit_false_resumes() {
    let request: DispatcherPauseRequest =
        serde_json::from_value(serde_json::json!({ "paused": false, "actor": "me" })).unwrap();
    assert!(!request.paused);
    assert_eq!(request.actor.as_deref(), Some("me"));
}
