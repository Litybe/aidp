use super::*;

#[test]
fn the_reply_has_the_shape_claude_code_reads() {
    let reply = HookReply::new("SubagentStart", "contexte".to_string());
    let value = serde_json::to_value(&reply).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "SubagentStart",
                "additionalContext": "contexte"
            }
        })
    );
}

#[test]
fn the_compact_text_is_a_single_line() {
    let reply = HookReply::new("SessionStart", "a\nb".to_string());
    let text = serde_json::to_string(&reply).unwrap();
    assert!(!text.contains('\n'));
    assert!(text.starts_with("{\"hookSpecificOutput\":"));
}
