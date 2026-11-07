use super::{Engine, MessageRole, SessionEventKind, SessionStatus, StreamMessageType};
use serde_json::json;

#[test]
fn stream_message_type_user_serializes_to_user() {
    assert_eq!(
        serde_json::to_value(StreamMessageType::User).unwrap(),
        json!("user")
    );
}

#[test]
fn message_role_user_serializes_to_user() {
    assert_eq!(
        serde_json::to_value(MessageRole::User).unwrap(),
        json!("user")
    );
}

#[test]
fn session_event_kind_variants_serialize_to_their_snake_case_string() {
    let cases = [
        (SessionEventKind::WorkflowProgress, "workflow_progress"),
        (SessionEventKind::System, "system"),
        (SessionEventKind::Text, "text"),
        (SessionEventKind::Thinking, "thinking"),
        (SessionEventKind::Question, "question"),
        (SessionEventKind::ToolUse, "tool_use"),
        (SessionEventKind::ToolResult, "tool_result"),
        (SessionEventKind::TurnEnd, "turn_end"),
        (SessionEventKind::Raw, "raw"),
        (SessionEventKind::Log, "log"),
        (SessionEventKind::Error, "error"),
        (SessionEventKind::UserReply, "user_reply"),
        (SessionEventKind::UserMessage, "user_message"),
    ];
    for (variant, expected) in cases {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(expected));
        assert_eq!(variant.as_str(), expected);
    }
}

#[test]
fn session_status_variants_serialize_to_their_lowercase_name_and_parse_back() {
    let cases = [
        (SessionStatus::Queued, "queued"),
        (SessionStatus::Running, "running"),
        (SessionStatus::Done, "done"),
        (SessionStatus::Failed, "failed"),
        (SessionStatus::Cancelled, "cancelled"),
        (SessionStatus::Orphaned, "orphaned"),
    ];
    for (variant, expected) in cases {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(expected));
        assert_eq!(variant.as_str(), expected);
        assert_eq!(SessionStatus::parse(expected), Some(variant));
    }
    assert_eq!(SessionStatus::parse("unknown"), None);
}

const ENGINE_V7_NAME: &str = "v7";
const ENGINE_WORKFLOW_NAME: &str = "workflow";
const ENGINE_NATIVE_NAME: &str = "native";
const ENGINE_PROMPT_NAME: &str = "prompt";
const ENGINE_UNKNOWN_NAME: &str = "unknown";

#[test]
fn engine_parse_maps_each_name_and_the_legacy_alias() {
    let cases = [
        (ENGINE_V7_NAME, Some(Engine::Workflow)),
        (ENGINE_WORKFLOW_NAME, Some(Engine::Workflow)),
        (ENGINE_NATIVE_NAME, Some(Engine::Native)),
        (ENGINE_PROMPT_NAME, Some(Engine::Prompt)),
        (ENGINE_UNKNOWN_NAME, None),
        ("", None),
    ];
    for (name, expected) in cases {
        assert_eq!(Engine::parse(name), expected, "{name}");
    }
}

#[test]
fn engine_as_str_parses_back_for_every_current_engine() {
    for engine in [Engine::Workflow, Engine::Native, Engine::Prompt] {
        assert_eq!(Engine::parse(engine.as_str()), Some(engine));
    }
}

/// Each engine serializes to its wire name and reads back; the legacy `v7` reads as `Workflow`.
#[test]
fn engine_serde_round_trips_and_reads_the_legacy_alias_as_workflow() {
    for engine in [Engine::Workflow, Engine::Native, Engine::Prompt] {
        let value = serde_json::to_value(engine).unwrap();
        assert_eq!(value, json!(engine.as_str()));
        assert_eq!(serde_json::from_value::<Engine>(value).unwrap(), engine);
    }
    let legacy: Engine = serde_json::from_value(json!(ENGINE_V7_NAME)).unwrap();
    assert_eq!(legacy, Engine::Workflow);
    assert!(serde_json::from_value::<Engine>(json!(ENGINE_UNKNOWN_NAME)).is_err());
}

#[test]
fn native_reason_texts_cover_isolation_failures() {
    use crate::models::enums::native_reason::NativeReason;
    assert_eq!(
        NativeReason::IsolationFailed.as_str(),
        "could not prepare an isolated worktree for the task"
    );
    assert_eq!(
        NativeReason::ApplyConflict.to_string(),
        "its changes conflict with the work of an earlier task"
    );
}
