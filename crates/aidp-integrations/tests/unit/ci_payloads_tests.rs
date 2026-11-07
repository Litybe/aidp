use super::*;
use serde_json::json;

/// `HookWakePayload` serializes to the exact nested shape a Claude Code hook
/// wake reads: `hookSpecificOutput.continueConversation`/`additionalContext`.
#[test]
fn hook_wake_payload_matches_hook_specific_output_shape() {
    let payload = HookWakePayload::new(true, "PR #7 is red");
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(
        value,
        json!({
            "hookSpecificOutput": {
                "continueConversation": true,
                "additionalContext": "PR #7 is red",
            }
        })
    );
}

/// `SystemMessagePayload` serializes to a single `systemMessage` key.
#[test]
fn system_message_payload_matches_system_message_key() {
    let payload = SystemMessagePayload::new("PR #7 is red".to_string());
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value, json!({ "systemMessage": "PR #7 is red" }));
}

/// `RunInFlightFields::merge_into` adds both fields onto an existing object
/// without disturbing keys already present.
#[test]
fn run_in_flight_fields_merge_into_preserves_existing_keys() {
    let mut base = json!({ "checks": { "pending": [] } });
    RunInFlightFields {
        run_in_flight: true,
        run_in_flight_reason: Some("a run is in progress".to_string()),
    }
    .merge_into(&mut base);
    assert_eq!(
        base,
        json!({
            "checks": { "pending": [] },
            "run_in_flight": true,
            "run_in_flight_reason": "a run is in progress",
        })
    );
}

/// A `None` reason serializes to JSON `null`, not an absent key.
#[test]
fn run_in_flight_fields_merge_into_writes_null_for_absent_reason() {
    let mut base = json!({});
    RunInFlightFields {
        run_in_flight: false,
        run_in_flight_reason: None,
    }
    .merge_into(&mut base);
    assert_eq!(
        base,
        json!({ "run_in_flight": false, "run_in_flight_reason": null })
    );
}

/// `AckReply` serializes to a single `acked` count.
#[test]
fn ack_reply_matches_acked_key() {
    let payload = AckReply::new(3);
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value, json!({ "acked": 3 }));
}

/// `ResetReply` serializes both `reset` and `state_file` keys.
#[test]
fn reset_reply_matches_reset_and_state_file_keys() {
    let payload = ResetReply::new(true, "/tmp/state.json".to_string());
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(
        value,
        json!({ "reset": true, "state_file": "/tmp/state.json" })
    );
}
