use super::*;

const REFUSAL_REASON: &str = "graphify_bin must be a bare program name";
const MISSING_REASON: &str = "graphify CLI not found";
const OTHER_REASON: &str = "graphify exited with status 1";

/// A containment refusal is surfaced as the init-failing reason.
#[test]
fn containment_refusal_returns_reason() {
    let state =
        KnowledgeGraphState::failed_with_kind(REFUSAL_REASON, KgFailureKind::ContainmentRefused);
    assert_eq!(state.containment_refusal().as_deref(), Some(REFUSAL_REASON));
}

/// A missing binary stays non-blocking even when its text names a setting.
#[test]
fn binary_missing_is_not_a_refusal() {
    let state = KnowledgeGraphState::failed_with_kind(MISSING_REASON, KgFailureKind::BinaryMissing);
    assert_eq!(state.containment_refusal(), None);
}

/// Untyped failures and non-failed states never refuse.
#[test]
fn other_and_non_failed_states_are_not_refusals() {
    assert_eq!(
        KnowledgeGraphState::failed(OTHER_REASON).containment_refusal(),
        None
    );
    let mut skipped = KnowledgeGraphState::skipped(REFUSAL_REASON);
    skipped.failure_kind = Some(KgFailureKind::ContainmentRefused);
    assert_eq!(skipped.containment_refusal(), None);
}

/// A state without a failure kind serializes without the key and still deserializes.
#[test]
fn failure_kind_is_omitted_when_absent() {
    let json = serde_json::to_string(&KnowledgeGraphState::failed(OTHER_REASON)).unwrap();
    assert!(!json.contains("failure_kind"));
    let back: KnowledgeGraphState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.failure_kind, None);
}
