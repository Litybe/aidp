use crate::models::enums::envelope_status::EnvelopeStatus;
use crate::models::enums::run_status::RunStatus;
use crate::models::enums::workflow_status::WorkflowStatus;

const UNRECOGNISED: &str = "not-a-status";

/// Every workflow status survives a parse and as_str round trip.
#[test]
fn workflow_status_round_trips_through_its_wire_value() {
    for status in [
        WorkflowStatus::Done,
        WorkflowStatus::Paused,
        WorkflowStatus::AwaitingGate,
        WorkflowStatus::AwaitingPlan,
    ] {
        assert_eq!(WorkflowStatus::parse(status.as_str()), status);
    }
}

/// An unrecognised workflow status falls back to Unknown, also through serde.
#[test]
fn workflow_status_falls_back_to_unknown() {
    assert_eq!(WorkflowStatus::parse(UNRECOGNISED), WorkflowStatus::Unknown);
    let parsed: WorkflowStatus = serde_json::from_str("\"not-a-status\"").unwrap();
    assert_eq!(parsed, WorkflowStatus::Unknown);
}

/// The status is read from a result object and is absent when the key is missing.
#[test]
fn workflow_status_is_read_from_a_result_object() {
    let result = serde_json::json!({ "status": "awaiting_gate" });
    assert_eq!(
        WorkflowStatus::from_result(&result),
        Some(WorkflowStatus::AwaitingGate)
    );
    assert_eq!(WorkflowStatus::from_result(&serde_json::json!({})), None);
}

/// Only completed and needs_review envelopes count as a success.
#[test]
fn envelope_status_is_success_only_for_completed_and_needs_review() {
    assert!(EnvelopeStatus::Completed.is_success());
    assert!(EnvelopeStatus::NeedsReview.is_success());
    assert!(!EnvelopeStatus::Blocked.is_success());
    assert!(!EnvelopeStatus::Failed.is_success());
    assert!(!EnvelopeStatus::Unknown.is_success());
}

/// Envelope statuses round-trip and an unrecognised value becomes the default Unknown.
#[test]
fn envelope_status_round_trips_and_defaults_to_unknown() {
    for status in [
        EnvelopeStatus::Completed,
        EnvelopeStatus::NeedsReview,
        EnvelopeStatus::Blocked,
        EnvelopeStatus::Failed,
    ] {
        assert_eq!(EnvelopeStatus::parse(status.as_str()), status);
    }
    assert_eq!(EnvelopeStatus::parse(UNRECOGNISED), EnvelopeStatus::Unknown);
    assert_eq!(EnvelopeStatus::default(), EnvelopeStatus::Unknown);
}

/// Display, as_str and the String conversion agree for every run status.
#[test]
fn run_status_string_forms_agree() {
    for (status, wire) in [
        (RunStatus::Failed, "failed"),
        (RunStatus::Done, "done"),
        (RunStatus::InProgress, "in_progress"),
        (RunStatus::AwaitingGate, "awaiting_gate"),
        (RunStatus::Pending, "pending"),
        (RunStatus::Paused, "paused"),
    ] {
        assert_eq!(status.as_str(), wire);
        assert_eq!(status.to_string(), wire);
        assert_eq!(String::from(status), wire);
    }
}
