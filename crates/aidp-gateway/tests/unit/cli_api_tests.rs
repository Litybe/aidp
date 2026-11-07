use super::*;

#[test]
fn parse_inputs_trims_keys_and_values() {
    let map = parse_inputs(vec![" track = deep ".to_string()]).unwrap();
    assert_eq!(map["track"], json!("deep"));
}

#[test]
fn parse_inputs_rejects_a_pair_without_equals() {
    let error = parse_inputs(vec!["track".to_string()]).unwrap_err();
    assert!(error.to_string().contains("key=value"));
}

#[test]
fn start_requires_a_template_or_a_prompt() {
    let params = StartParams {
        project: "p".to_string(),
        ..StartParams::default()
    };
    let error = start(&None, params).unwrap_err();
    assert!(error.to_string().contains("--template"));
}

#[test]
fn raw_reply_renders_strings_without_quotes() {
    assert_eq!(Reply::raw(json!("ok")).text(), "ok");
}

#[test]
fn started_session_reply_names_the_session() {
    let reply = Reply::new(
        json!({ "id": "s1", "templateId": "dev", "projectName": "proj" }),
        TextView::StartedSession,
    );
    assert_eq!(reply.text(), "session s1 started (dev on proj)");
}
